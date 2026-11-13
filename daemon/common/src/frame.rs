use std::io::{self, Read, Write};

pub const MAX_FRAME: usize = 1 << 20;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("connection closed")]
    Closed,
    #[error("frame length {0} is outside 1..=1048576")]
    BadLength(usize),
    #[error("frame is not valid UTF-8 JSON: {0}")]
    BadPayload(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub fn check_len(len: usize) -> Result<(), FrameError> {
    if len == 0 || len > MAX_FRAME {
        Err(FrameError::BadLength(len))
    } else {
        Ok(())
    }
}

pub fn encode(payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    check_len(payload.len())?;
    let mut out = Vec::with_capacity(payload.len() + 4);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

pub fn read_frame<R: Read>(r: &mut R, buf: &mut Vec<u8>) -> Result<(), FrameError> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Err(FrameError::Closed),
        Err(e) => return Err(e.into()),
    }
    let n = u32::from_be_bytes(len) as usize;
    check_len(n)?;
    buf.clear();
    buf.resize(n, 0);
    r.read_exact(buf).map_err(|e| {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            FrameError::Closed
        } else {
            e.into()
        }
    })
}

pub fn write_frame<W: Write>(w: &mut W, payload: &[u8]) -> Result<(), FrameError> {
    check_len(payload.len())?;
    w.write_all(&(payload.len() as u32).to_be_bytes())?;
    w.write_all(payload)?;
    w.flush()?;
    Ok(())
}

pub fn parse_object(buf: &[u8]) -> Result<serde_json::Map<String, serde_json::Value>, FrameError> {
    let text = std::str::from_utf8(buf).map_err(|e| FrameError::BadPayload(e.to_string()))?;
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Object(m)) => Ok(m),
        Ok(_) => Err(FrameError::BadPayload("not an object".into())),
        Err(e) => Err(FrameError::BadPayload(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut wire = Vec::new();
        write_frame(&mut wire, br#"{"a":1}"#).unwrap();
        let mut cur = io::Cursor::new(wire);
        let mut buf = Vec::new();
        read_frame(&mut cur, &mut buf).unwrap();
        assert_eq!(parse_object(&buf).unwrap()["a"], 1);
        assert!(matches!(read_frame(&mut cur, &mut buf), Err(FrameError::Closed)));
    }

    #[test]
    fn rejects_bad_lengths() {
        let mut cur = io::Cursor::new(vec![0, 0, 0, 0]);
        assert!(matches!(read_frame(&mut cur, &mut Vec::new()), Err(FrameError::BadLength(0))));
        let mut cur = io::Cursor::new(((MAX_FRAME + 1) as u32).to_be_bytes().to_vec());
        assert!(matches!(read_frame(&mut cur, &mut Vec::new()), Err(FrameError::BadLength(_))));
        assert!(encode(&vec![b'x'; MAX_FRAME + 1]).is_err());
    }

    #[test]
    fn rejects_non_objects_and_bad_utf8() {
        assert!(parse_object(b"[1,2]").is_err());
        assert!(parse_object(&[0xff, 0xfe]).is_err());
        assert!(parse_object(b"{").is_err());
    }

    #[test]
    fn truncated_payload_is_closed() {
        let mut wire = 10u32.to_be_bytes().to_vec();
        wire.extend_from_slice(b"abc");
        let mut cur = io::Cursor::new(wire);
        assert!(matches!(read_frame(&mut cur, &mut Vec::new()), Err(FrameError::Closed)));
    }
}
