//! Bounded IPP/1.1 wire codec for a PDF-only proof printer.
//! RFC 8010: https://www.rfc-editor.org/rfc/rfc8010
pub const PRINT_JOB: u16 = 0x0002;
pub const VALIDATE_JOB: u16 = 0x0004;
pub const GET_JOB_ATTRIBUTES: u16 = 0x0009;
pub const GET_JOBS: u16 = 0x000a;
pub const CANCEL_JOB: u16 = 0x0008;
pub const GET_PRINTER_ATTRIBUTES: u16 = 0x000b;
pub const OK: u16 = 0x0000;
pub const BAD_REQUEST: u16 = 0x0400;
pub const NOT_FOUND: u16 = 0x0406;
pub const DOCUMENT_FORMAT_NOT_SUPPORTED: u16 = 0x040a;
pub const ATTRIBUTES_NOT_SUPPORTED: u16 = 0x040b;
pub const FORMAT_ERROR: u16 = 0x0411;
pub const OPERATION_NOT_SUPPORTED: u16 = 0x0501;
pub const INTERNAL_ERROR: u16 = 0x0500;

#[derive(Debug, thiserror::Error)]
#[error("invalid_ipp_message")]
pub struct ParseError;
#[derive(Debug, Clone)]
pub struct Attribute {
    pub group: u8,
    pub tag: u8,
    pub name: String,
    pub value: Vec<u8>,
}
#[derive(Debug)]
pub struct Request<'a> {
    pub version: [u8; 2],
    pub operation: u16,
    pub request_id: u32,
    pub attributes: Vec<Attribute>,
    pub document: &'a [u8],
}
impl<'a> Request<'a> {
    pub fn parse(input: &'a [u8]) -> Result<Self, ParseError> {
        if input.len() < 9 {
            return Err(ParseError);
        }
        let version = [input[0], input[1]];
        if !matches!(version, [1, 1] | [2, 0]) {
            return Err(ParseError);
        }
        let operation = u16::from_be_bytes([input[2], input[3]]);
        let request_id = u32::from_be_bytes(input[4..8].try_into().map_err(|_| ParseError)?);
        if request_id == 0 {
            return Err(ParseError);
        }
        let mut offset = 8usize;
        let mut group = 0u8;
        let mut attributes = Vec::new();
        let mut current = String::new();
        loop {
            let tag = *input.get(offset).ok_or(ParseError)?;
            offset += 1;
            match tag {
                0x03 => break,
                0x01 | 0x02 if group <= tag => {
                    group = tag;
                    current.clear();
                }
                0x04 if group <= tag => {
                    group = tag;
                    current.clear();
                }
                0x05 => return Err(ParseError),
                0x10..=0x5f if group != 0 => {
                    if attributes.len() >= 128 {
                        return Err(ParseError);
                    }
                    let name_len = take_u16(input, &mut offset)? as usize;
                    if name_len > 255 {
                        return Err(ParseError);
                    }
                    let name = if name_len == 0 {
                        if current.is_empty() {
                            return Err(ParseError);
                        }
                        current.clone()
                    } else {
                        let bytes = take(input, &mut offset, name_len)?;
                        let value = std::str::from_utf8(bytes).map_err(|_| ParseError)?;
                        if !value
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                        {
                            return Err(ParseError);
                        }
                        current = value.to_string();
                        current.clone()
                    };
                    let len = take_u16(input, &mut offset)? as usize;
                    if len > 4096 {
                        return Err(ParseError);
                    }
                    let value = take(input, &mut offset, len)?.to_vec();
                    attributes.push(Attribute {
                        group,
                        tag,
                        name,
                        value,
                    });
                }
                _ => return Err(ParseError),
            }
        }
        if attributes.first().is_none_or(|a| a.group != 0x01) {
            return Err(ParseError);
        }
        Ok(Self {
            version,
            operation,
            request_id,
            attributes,
            document: &input[offset..],
        })
    }
    pub fn one(&self, group: u8, name: &str) -> Option<&Attribute> {
        let mut values = self
            .attributes
            .iter()
            .filter(|a| a.group == group && a.name == name);
        let first = values.next()?;
        if values.next().is_some() {
            None
        } else {
            Some(first)
        }
    }
    pub fn text(&self, group: u8, name: &str, tag: u8) -> Option<&str> {
        let a = self.one(group, name)?;
        if a.tag != tag {
            return None;
        }
        std::str::from_utf8(&a.value).ok()
    }
    pub fn integer(&self, group: u8, name: &str) -> Option<i32> {
        let a = self.one(group, name)?;
        if a.tag != 0x21 || a.value.len() != 4 {
            return None;
        }
        Some(i32::from_be_bytes(a.value[..4].try_into().ok()?))
    }
    pub fn resolution_dpi(&self, group: u8, name: &str) -> Option<u32> {
        let a = self.one(group, name)?;
        if a.tag != 0x32 || a.value.len() != 9 || a.value[8] != 3 {
            return None;
        }
        let x = u32::from_be_bytes(a.value[0..4].try_into().ok()?);
        let y = u32::from_be_bytes(a.value[4..8].try_into().ok()?);
        (x == y).then_some(x)
    }
    pub fn wants(&self, name: &str, group_name: &str) -> bool {
        let requested: Vec<_> = self
            .attributes
            .iter()
            .filter(|a| a.group == 1 && a.name == "requested-attributes")
            .collect();
        requested.is_empty()
            || requested.iter().any(|a| {
                a.value == b"all" || a.value == group_name.as_bytes() || a.value == name.as_bytes()
            })
    }
    pub fn validate_operation(&self) -> Result<(), u16> {
        if self
            .attributes
            .first()
            .is_none_or(|a| a.name != "attributes-charset")
            || self
                .attributes
                .get(1)
                .is_none_or(|a| a.name != "attributes-natural-language")
        {
            return Err(BAD_REQUEST);
        }
        if self.text(1, "attributes-charset", 0x47) != Some("utf-8")
            || self.text(1, "attributes-natural-language", 0x48).is_none()
        {
            return Err(BAD_REQUEST);
        }
        let printer_uri = self.text(1, "printer-uri", 0x45);
        let job_uri = self.text(1, "job-uri", 0x45);
        if matches!(self.operation, GET_JOB_ATTRIBUTES | CANCEL_JOB) {
            if (printer_uri.is_some() == job_uri.is_some())
                || (printer_uri.is_some() && self.integer(1, "job-id").is_none())
                || (job_uri.is_some() && self.one(1, "job-id").is_some())
                || job_uri.is_some_and(|uri| {
                    !uri.starts_with("ipp://") || !uri.contains("/ipp/print/jobs/")
                })
            {
                return Err(BAD_REQUEST);
            }
        } else if printer_uri.is_none() || job_uri.is_some() {
            return Err(BAD_REQUEST);
        }
        if printer_uri.is_some_and(|uri| !uri.starts_with("ipp://") || !uri.ends_with("/ipp/print"))
        {
            return Err(BAD_REQUEST);
        }
        if self.attributes.iter().any(|a| a.group != 1 && a.group != 2) {
            return Err(ATTRIBUTES_NOT_SUPPORTED);
        }
        for a in &self.attributes {
            let expected = match a.name.as_str() {
                "attributes-charset" => 0x47,
                "attributes-natural-language" => 0x48,
                "printer-uri" | "job-uri" => 0x45,
                "requesting-user-name" | "job-name" => 0x42,
                "document-format" => 0x49,
                "job-id" => 0x21,
                "requested-attributes" => 0x44,
                "printer-resolution" => 0x32,
                "print-color-mode" => 0x44,
                "limit" => 0x21,
                "which-jobs" => 0x44,
                "my-jobs" => 0x22,
                _ => return Err(ATTRIBUTES_NOT_SUPPORTED),
            };
            if a.tag != expected
                || (expected == 0x21 && a.value.len() != 4)
                || (expected == 0x32 && a.value.len() != 9)
                || (expected == 0x22 && (a.value.len() != 1 || a.value[0] > 1))
            {
                return Err(BAD_REQUEST);
            }
            if expected != 0x21
                && expected != 0x32
                && expected != 0x22
                && (a.value.is_empty()
                    || a.value.contains(&0)
                    || std::str::from_utf8(&a.value).is_err())
            {
                return Err(BAD_REQUEST);
            }
            if a.name != "requested-attributes"
                && self
                    .attributes
                    .iter()
                    .filter(|other| other.group == a.group && other.name == a.name)
                    .count()
                    != 1
            {
                return Err(BAD_REQUEST);
            }
        }
        match self.operation {
            PRINT_JOB | VALIDATE_JOB => {
                if self.operation == PRINT_JOB && !self.document.starts_with(b"%PDF-") {
                    return Err(FORMAT_ERROR);
                }
                if self.operation == VALIDATE_JOB && !self.document.is_empty() {
                    return Err(BAD_REQUEST);
                }
                if self.attributes.iter().any(|a| {
                    (a.group == 2
                        && !matches!(a.name.as_str(), "printer-resolution" | "print-color-mode"))
                        || (a.group == 1
                            && !matches!(
                                a.name.as_str(),
                                "attributes-charset"
                                    | "attributes-natural-language"
                                    | "printer-uri"
                                    | "requesting-user-name"
                                    | "job-name"
                                    | "document-format"
                            ))
                }) {
                    return Err(ATTRIBUTES_NOT_SUPPORTED);
                }
                if self.one(2, "printer-resolution").is_some()
                    && !matches!(
                        self.resolution_dpi(2, "printer-resolution"),
                        Some(300 | 600)
                    )
                {
                    return Err(ATTRIBUTES_NOT_SUPPORTED);
                }
                if let Some(a) = self.one(2, "print-color-mode")
                    && a.value != b"color"
                    && a.value != b"monochrome"
                {
                    return Err(ATTRIBUTES_NOT_SUPPORTED);
                }
                match self.text(1, "document-format", 0x49) {
                    Some("application/pdf") | None => Ok(()),
                    _ => Err(DOCUMENT_FORMAT_NOT_SUPPORTED),
                }
            }
            GET_PRINTER_ATTRIBUTES => {
                if self.document.is_empty()
                    && self.attributes.iter().all(|a| a.group == 1)
                    && self.attributes.iter().all(|a| {
                        matches!(
                            a.name.as_str(),
                            "attributes-charset"
                                | "attributes-natural-language"
                                | "printer-uri"
                                | "requesting-user-name"
                                | "document-format"
                                | "requested-attributes"
                        )
                    })
                {
                    Ok(())
                } else {
                    Err(BAD_REQUEST)
                }
            }
            GET_JOBS => {
                if !self.document.is_empty()
                    || self.attributes.iter().any(|a| {
                        a.group != 1
                            || !matches!(
                                a.name.as_str(),
                                "attributes-charset"
                                    | "attributes-natural-language"
                                    | "printer-uri"
                                    | "requesting-user-name"
                                    | "limit"
                                    | "which-jobs"
                                    | "my-jobs"
                                    | "requested-attributes"
                            )
                    })
                    || self.integer(1, "limit").is_some_and(|n| n < 1)
                {
                    return Err(BAD_REQUEST);
                }
                if self
                    .text(1, "which-jobs", 0x44)
                    .is_some_and(|value| value != "completed" && value != "not-completed")
                    || self.one(1, "my-jobs").is_some_and(|a| a.value == [1])
                {
                    return Err(ATTRIBUTES_NOT_SUPPORTED);
                }
                Ok(())
            }
            GET_JOB_ATTRIBUTES | CANCEL_JOB => {
                if !self.document.is_empty()
                    || self.integer(1, "job-id").is_some_and(|n| n < 1)
                    || (job_uri.is_none() && self.integer(1, "job-id").is_none())
                    || self.attributes.iter().any(|a| {
                        a.group == 2
                            || !matches!(
                                a.name.as_str(),
                                "attributes-charset"
                                    | "attributes-natural-language"
                                    | "printer-uri"
                                    | "job-uri"
                                    | "requesting-user-name"
                                    | "job-id"
                                    | "requested-attributes"
                            )
                            || (self.operation == CANCEL_JOB && a.name == "requested-attributes")
                    })
                {
                    Err(BAD_REQUEST)
                } else {
                    Ok(())
                }
            }
            _ => Err(OPERATION_NOT_SUPPORTED),
        }
    }
}
fn take<'a>(input: &'a [u8], offset: &mut usize, len: usize) -> Result<&'a [u8], ParseError> {
    let end = offset.checked_add(len).ok_or(ParseError)?;
    let bytes = input.get(*offset..end).ok_or(ParseError)?;
    *offset = end;
    Ok(bytes)
}
fn take_u16(input: &[u8], offset: &mut usize) -> Result<u16, ParseError> {
    Ok(u16::from_be_bytes(
        take(input, offset, 2)?.try_into().map_err(|_| ParseError)?,
    ))
}
pub struct Response {
    bytes: Vec<u8>,
}
impl Response {
    pub fn new(version: [u8; 2], status: u16, request_id: u32) -> Self {
        let mut bytes = Vec::with_capacity(512);
        bytes.extend_from_slice(&version);
        bytes.extend_from_slice(&status.to_be_bytes());
        bytes.extend_from_slice(&request_id.to_be_bytes());
        bytes.push(1);
        Self { bytes }
    }
    pub fn attribute(&mut self, tag: u8, name: &str, value: &[u8]) {
        assert!(name.len() <= u16::MAX as usize && value.len() <= u16::MAX as usize);
        self.bytes.push(tag);
        self.bytes
            .extend_from_slice(&(name.len() as u16).to_be_bytes());
        self.bytes.extend_from_slice(name.as_bytes());
        self.bytes
            .extend_from_slice(&(value.len() as u16).to_be_bytes());
        self.bytes.extend_from_slice(value);
    }
    pub fn group(&mut self, tag: u8) {
        self.bytes.push(tag);
    }
    pub fn additional(&mut self, tag: u8, value: &[u8]) {
        self.bytes.push(tag);
        self.bytes.extend_from_slice(&0u16.to_be_bytes());
        self.bytes
            .extend_from_slice(&(value.len() as u16).to_be_bytes());
        self.bytes.extend_from_slice(value);
    }
    pub fn text(&mut self, tag: u8, name: &str, value: &str) {
        self.attribute(tag, name, value.as_bytes());
    }
    pub fn integer(&mut self, name: &str, value: i32) {
        self.attribute(0x21, name, &value.to_be_bytes());
    }
    pub fn resolution_dpi(&mut self, name: &str, dpi: u32) {
        let mut value = [0u8; 9];
        value[0..4].copy_from_slice(&dpi.to_be_bytes());
        value[4..8].copy_from_slice(&dpi.to_be_bytes());
        value[8] = 3;
        self.attribute(0x32, name, &value);
    }
    pub fn finish(mut self) -> Vec<u8> {
        self.bytes.push(3);
        self.bytes
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_document_boundary_and_rejects_truncation() {
        let mut msg = vec![1, 1, 0, 2, 0, 0, 0, 7, 1];
        for (tag, name, val) in [
            (0x47, "attributes-charset", "utf-8"),
            (0x48, "attributes-natural-language", "en"),
            (0x45, "printer-uri", "ipp://localhost/ipp/print"),
            (0x49, "document-format", "application/pdf"),
        ] {
            msg.push(tag);
            msg.extend_from_slice(&(name.len() as u16).to_be_bytes());
            msg.extend_from_slice(name.as_bytes());
            msg.extend_from_slice(&(val.len() as u16).to_be_bytes());
            msg.extend_from_slice(val.as_bytes());
        }
        msg.push(3);
        msg.extend_from_slice(b"%PDF-1.4\n");
        let req = Request::parse(&msg).unwrap();
        assert_eq!(req.document, b"%PDF-1.4\n");
        assert!(req.validate_operation().is_ok());
        for len in 0..msg.len() - 10 {
            assert!(Request::parse(&msg[..len]).is_err());
        }
    }
}
