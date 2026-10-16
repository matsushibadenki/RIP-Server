//! Length-delimited, bounded adapter stdout. Stderr is never a progress source.
use super::*;

async fn exact<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    bytes: &mut [u8],
) -> Result<(), InterpreterError> {
    reader.read_exact(bytes).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            InterpreterError::Rejected("interpreter_failed")
        } else {
            error.into()
        }
    })?;
    Ok(())
}

pub(super) async fn read_page_stream<R, F, Fut>(
    reader: &mut R,
    output: &Path,
    max_pages: u32,
    max_bytes: u64,
    progress: &mut F,
) -> Result<Vec<PathBuf>, InterpreterError>
where
    R: tokio::io::AsyncRead + Unpin,
    F: FnMut(u32) -> Fut + Send,
    Fut: std::future::Future<Output = Result<(), InterpreterError>> + Send,
{
    let mut magic = [0u8; 13];
    // Protocol magic is deliberately not an engine diagnostic line.
    exact(reader, &mut magic).await?;
    if &magic != b"JRIP-PAGES-1\n" {
        return Err(InterpreterError::Rejected("invalid_output_protocol"));
    }
    let mut pages = Vec::new();
    let mut total = 0u64;
    loop {
        let mut frame = [0u8; 12];
        exact(reader, &mut frame).await?;
        let page = u32::from_be_bytes(frame[..4].try_into().unwrap());
        let size = u64::from_be_bytes(frame[4..].try_into().unwrap());
        if page == 0 && size == 0 {
            if pages.is_empty() {
                return Err(InterpreterError::Rejected("empty_output"));
            }
            let mut tail = [0u8; 1];
            if reader.read(&mut tail).await? != 0 {
                return Err(InterpreterError::Rejected("invalid_output"));
            }
            return Ok(pages);
        }
        if page != pages.len() as u32 + 1 || page > max_pages || size < 8 {
            return Err(InterpreterError::Rejected("invalid_output"));
        }
        total = total
            .checked_add(size)
            .filter(|n| *n <= max_bytes)
            .ok_or(InterpreterError::Rejected("resource_limit"))?;
        let path = output.join(format!("page-{page:06}.tiff"));
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await?;
        let mut header = [0u8; 8];
        exact(reader, &mut header).await?;
        if header[..4] != *b"II\x2a\x00" && header[..4] != *b"MM\x00\x2a" {
            return Err(InterpreterError::Rejected("invalid_tiff"));
        }
        file.write_all(&header).await?;
        let mut remaining = size - 8;
        let mut buffer = [0u8; 65536];
        while remaining > 0 {
            let count = remaining.min(buffer.len() as u64) as usize;
            exact(reader, &mut buffer[..count]).await?;
            file.write_all(&buffer[..count]).await?;
            remaining -= count as u64;
        }
        file.flush().await?;
        file.sync_all().await?;
        drop(file);
        pages.push(path);
        progress(page).await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(page: u32, data: &[u8]) -> Vec<u8> {
        let mut bytes = page.to_be_bytes().to_vec();
        bytes.extend_from_slice(&(data.len() as u64).to_be_bytes());
        bytes.extend_from_slice(data);
        bytes
    }
    const TIFF: &[u8] = b"II\x2a\x00\x08\x00\x00\x00";

    #[tokio::test]
    async fn observes_first_page_before_producer_can_finish() {
        let dir = tempfile::tempdir().unwrap();
        let (mut writer, mut reader) = tokio::io::duplex(32);
        let resume = std::sync::Arc::new(tokio::sync::Notify::new());
        let signal = resume.clone();
        let producer = tokio::spawn(async move {
            writer.write_all(b"JRIP-PAGES-1\n").await.unwrap();
            writer.write_all(&frame(1, TIFF)).await.unwrap();
            signal.notified().await;
            writer.write_all(&frame(2, TIFF)).await.unwrap();
            writer.write_all(&[0u8; 12]).await.unwrap();
        });
        let mut observed = vec![];
        let mut callback = |page| {
            assert!(dir.path().join(format!("page-{page:06}.tiff")).exists());
            assert!(
                !dir.path()
                    .join(format!("page-{:06}.tiff", page + 1))
                    .exists()
            );
            observed.push(page);
            if page == 1 {
                resume.notify_one();
            }
            async { Ok(()) }
        };
        let pages = tokio::time::timeout(
            Duration::from_secs(3),
            read_page_stream(&mut reader, dir.path(), 2, 1024, &mut callback),
        )
        .await
        .unwrap()
        .unwrap();
        producer.await.unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(observed, vec![1, 2]);
    }

    #[tokio::test]
    async fn rejects_unbounded_malformed_and_incomplete_frames_without_false_progress() {
        let mut bad_cases = vec![b"arbitrary engine log".to_vec()];
        for body in [
            frame(2, TIFF),
            frame(1, b"notatiff"),
            frame(1, &TIFF[..4]),
            frame(1, TIFF)[..16].to_vec(),
            [0u8; 12].to_vec(),
        ] {
            let mut stream = b"JRIP-PAGES-1\n".to_vec();
            stream.extend(body);
            bad_cases.push(stream);
        }
        let mut huge = b"JRIP-PAGES-1\n".to_vec();
        huge.extend(1u32.to_be_bytes());
        huge.extend(u64::MAX.to_be_bytes());
        bad_cases.push(huge);
        for bytes in bad_cases {
            let dir = tempfile::tempdir().unwrap();
            let mut observed = vec![];
            let result =
                read_page_stream(&mut bytes.as_slice(), dir.path(), 2, 1024, &mut |page| {
                    observed.push(page);
                    async { Ok(()) }
                })
                .await;
            assert!(result.is_err());
            assert!(observed.is_empty());
        }
        // A valid observation survives a truncated later page, but no success is returned.
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = b"JRIP-PAGES-1\n".to_vec();
        bytes.extend(frame(1, TIFF));
        bytes.extend(frame(2, TIFF)[..16].iter());
        let mut observed = vec![];
        assert!(
            read_page_stream(&mut bytes.as_slice(), dir.path(), 2, 1024, &mut |page| {
                observed.push(page);
                async { Ok(()) }
            })
            .await
            .is_err()
        );
        assert_eq!(observed, vec![1]);
    }

    #[tokio::test]
    async fn observer_failure_stops_stream_consumption() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = b"JRIP-PAGES-1\n".to_vec();
        bytes.extend(frame(1, TIFF));
        bytes.extend(frame(2, TIFF));
        bytes.extend([0u8; 12]);
        let error = read_page_stream(&mut bytes.as_slice(), dir.path(), 2, 1024, &mut |_| async {
            Err(InterpreterError::Rejected("cancelled"))
        })
        .await
        .err()
        .unwrap();
        assert_eq!(error.to_string(), "cancelled");
        assert!(!dir.path().join("page-000002.tiff").exists());
    }
}
