use crate::error::{ErrorKind, RemoteOpsError, Result};

const CURSOR_PREFIX: &str = "seq:";

/// 将内部序号编码为 opaque cursor；调用方不依赖 byte offset。
pub fn encode_cursor(seq: u64) -> String {
    format!("{CURSOR_PREFIX}{seq}")
}

pub fn decode_cursor(cursor: Option<&str>) -> Result<u64> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    let Some(raw) = cursor.strip_prefix(CURSOR_PREFIX) else {
        return Err(RemoteOpsError::Remote(
            ErrorKind::ProtocolError,
            "cursor has invalid prefix".to_string(),
        ));
    };
    raw.parse::<u64>().map_err(|err| {
        RemoteOpsError::Remote(
            ErrorKind::ProtocolError,
            format!("cursor has invalid sequence: {err}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_cursor() {
        let cursor = encode_cursor(42);
        assert_eq!(decode_cursor(Some(&cursor)).unwrap(), 42);
        assert_eq!(decode_cursor(None).unwrap(), 0);
    }
}
