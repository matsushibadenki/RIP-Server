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

#[cfg(test)]
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
    Ok(read_selected_page_stream(
        reader,
        output,
        max_pages,
        max_bytes,
        &rip_core::pages::RenderPages::default(),
        progress,
    )
    .await?
    .pages)
}

pub(super) async fn read_selected_page_stream<R, F, Fut>(
    reader: &mut R,
    output: &Path,
    max_pages: u32,
    max_bytes: u64,
    selection: &rip_core::pages::RenderPages,
    progress: &mut F,
) -> Result<RenderedDocument, InterpreterError>
where
    R: tokio::io::AsyncRead + Unpin,
    F: FnMut(u32) -> Fut + Send,
    Fut: std::future::Future<Output = Result<(), InterpreterError>> + Send,
{
    let mut magic = [0u8; 13];
    // Protocol magic is deliberately not an engine diagnostic line.
    exact(reader, &mut magic).await?;
    if (selection.is_all() && &magic != b"JRIP-PAGES-1\n")
        || (!selection.is_all() && &magic != b"JRIP-PAGES-2\n")
    {
        return Err(InterpreterError::Rejected("invalid_output_protocol"));
    }
    let source_page_count = if selection.is_all() {
        None
    } else {
        let mut count = [0; 4];
        exact(reader, &mut count).await?;
        let count = u32::from_be_bytes(count);
        if count > max_pages || selection.indices(count).is_err() {
            return Err(InterpreterError::Rejected("page_range_out_of_bounds"));
        }
        Some(count)
    };
    let expected =
        source_page_count.map(|total| selection.indices(total).expect("checked indices").len());
    let mut pages = Vec::new();
    let mut total = 0u64;
    loop {
        let mut frame = [0u8; 12];
        exact(reader, &mut frame).await?;
        let page = u32::from_be_bytes(frame[..4].try_into().unwrap());
        let size = u64::from_be_bytes(frame[4..].try_into().unwrap());
        if page == 0 && size == 0 {
            if pages.is_empty() && expected != Some(0) {
                return Err(InterpreterError::Rejected("empty_output"));
            }
            let mut tail = [0u8; 1];
            if reader.read(&mut tail).await? != 0 {
                return Err(InterpreterError::Rejected("invalid_output"));
            }
            if expected.is_some_and(|n| n != pages.len()) {
                return Err(InterpreterError::Rejected("source_page_count_mismatch"));
            }
            return Ok(RenderedDocument {
                pages,
                source_page_count,
            });
        }
        if page != pages.len() as u32 + 1
            || page > max_pages
            || size < 8
            || expected.is_some_and(|n| page as usize > n)
        {
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

#[cfg(test)]
mod selection_tests {
    use super::*;
    #[tokio::test]
    async fn requires_bound_source_count_and_exact_selected_frames() {
        let selection: rip_core::pages::RenderPages =
            rip_core::pages::PageRanges::new(vec![rip_core::pages::PageRange {
                first: 2,
                last: 2,
            }])
            .unwrap()
            .into();
        let frame = [
            1u32.to_be_bytes().as_slice(),
            8u64.to_be_bytes().as_slice(),
            b"II\x2a\x00\x08\x00\x00\x00",
        ]
        .concat();
        let valid = [
            b"JRIP-PAGES-2\n".as_slice(),
            2u32.to_be_bytes().as_slice(),
            &frame,
            &[0; 12],
        ]
        .concat();
        for (bytes, ok) in [
            (valid.clone(), true),
            (
                [b"JRIP-PAGES-1\n".as_slice(), &frame, &[0; 12]].concat(),
                false,
            ),
            (
                [
                    b"JRIP-PAGES-2\n".as_slice(),
                    1u32.to_be_bytes().as_slice(),
                    &frame,
                    &[0; 12],
                ]
                .concat(),
                false,
            ),
            (
                [
                    b"JRIP-PAGES-2\n".as_slice(),
                    101u32.to_be_bytes().as_slice(),
                    &frame,
                    &[0; 12],
                ]
                .concat(),
                false,
            ),
            (
                [
                    valid[..valid.len() - 12].as_ref(),
                    &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 8],
                ]
                .concat(),
                false,
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let result = read_selected_page_stream(
                &mut bytes.as_slice(),
                dir.path(),
                100,
                1024,
                &selection,
                &mut |_| async { Ok(()) },
            )
            .await;
            assert_eq!(result.is_ok(), ok);
            if let Ok(doc) = result {
                assert_eq!(doc.source_page_count, Some(2));
                assert_eq!(doc.pages.len(), 1);
            }
        }
    }
}

#[cfg(test)]
mod empty_selection_tests {
    use super::*;
    #[tokio::test]
    async fn zero_frames_require_a_proven_empty_ipp_intersection() {
        let bytes = [
            b"JRIP-PAGES-2\n".as_slice(),
            2u32.to_be_bytes().as_slice(),
            &[0; 12],
        ]
        .concat();
        for (selection, allowed) in [
            (
                rip_core::pages::RenderPages::Ipp {
                    ipp: rip_core::pages::IppPageRanges::new(vec![rip_core::pages::PageRange {
                        first: 3,
                        last: i32::MAX as u32,
                    }])
                    .unwrap(),
                },
                true,
            ),
            (
                rip_core::pages::RenderPages::Ipp {
                    ipp: rip_core::pages::IppPageRanges::new(vec![rip_core::pages::PageRange {
                        first: 2,
                        last: i32::MAX as u32,
                    }])
                    .unwrap(),
                },
                false,
            ),
            (
                rip_core::pages::PageRanges::new(vec![rip_core::pages::PageRange {
                    first: 2,
                    last: 2,
                }])
                .unwrap()
                .into(),
                false,
            ),
            (rip_core::pages::RenderPages::default(), false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let result = read_selected_page_stream(
                &mut bytes.as_slice(),
                dir.path(),
                100,
                1024,
                &selection,
                &mut |_| async { panic!("empty selection has no progress") },
            )
            .await;
            assert_eq!(result.is_ok(), allowed);
            if let Ok(rendered) = result {
                assert_eq!(rendered.source_page_count, Some(2));
                assert!(rendered.pages.is_empty());
            }
        }
    }
}
