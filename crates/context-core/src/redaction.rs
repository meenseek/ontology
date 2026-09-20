const REDACTION: &str = "[REDACTED]";

#[must_use]
pub fn redact_secrets(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for line in input.lines() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&redact_openai_like_tokens(&redact_sensitive_assignment(
            line,
        )));
    }
    if input.ends_with('\n') {
        output.push('\n');
    }
    output
}

fn redact_sensitive_assignment(line: &str) -> String {
    let Some(index) = separator_index(line) else {
        return line.to_owned();
    };
    let key = line[..index].trim().to_ascii_lowercase();
    if is_sensitive_key(&key) {
        let separator_end = index + 1;
        format!("{} {REDACTION}", &line[..separator_end])
    } else {
        line.to_owned()
    }
}

fn separator_index(line: &str) -> Option<usize> {
    [line.find(':'), line.find('=')].into_iter().flatten().min()
}

fn is_sensitive_key(key: &str) -> bool {
    [
        "api_key",
        "apikey",
        "cookie",
        "credential",
        "password",
        "private_key",
        "secret",
        "token",
    ]
    .iter()
    .any(|sensitive| key.contains(sensitive))
}

fn redact_openai_like_tokens(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(index) = rest.find("sk-") {
        output.push_str(&rest[..index]);
        let token_candidate = &rest[index..];
        let end = token_candidate
            .find(char::is_whitespace)
            .unwrap_or(token_candidate.len());
        let token = &token_candidate[..end];
        if token.len() >= 12 {
            output.push_str(REDACTION);
        } else {
            output.push_str(token);
        }
        rest = &token_candidate[end..];
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_sensitive_assignments() {
        let input = "api_key: synthetic-secret\nnormal: visible\npassword = synthetic-password";

        let redacted = redact_secrets(input);

        assert_eq!(
            redacted,
            "api_key: [REDACTED]\nnormal: visible\npassword = [REDACTED]"
        );
    }

    #[test]
    fn redacts_openai_like_token_words() {
        let input = "token value sk-synthetic123456789 stays private";

        let redacted = redact_secrets(input);

        assert_eq!(redacted, "token value [REDACTED] stays private");
    }
}
