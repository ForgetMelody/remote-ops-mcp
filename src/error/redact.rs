/// 对外输出和日志中的敏感字段脱敏。
pub fn redact_secret(input: &str) -> String {
    let mut output = input.to_owned();
    for key in ["password", "passwd", "token", "secret", "private_key"] {
        output = redact_key_value(&output, key);
    }
    output
}

fn redact_key_value(input: &str, key: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(pos) = lower.find(key)
            && lower[pos + key.len()..]
                .trim_start()
                .starts_with(['=', ':'])
        {
            out.push_str(&line[..pos + key.len()]);
            out.push_str("=<redacted>");
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !input.ends_with('\n') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_simple_key_value_lines() {
        let redacted = redact_secret("password: abc\ntoken=def\nkeep=1");
        assert!(redacted.contains("password=<redacted>"));
        assert!(redacted.contains("token=<redacted>"));
        assert!(redacted.contains("keep=1"));
    }
}
