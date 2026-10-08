const REDACTION: &str = "[REDACTED]";

#[must_use]
pub fn redact_secrets(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for line in input.lines() {
        if !output.is_empty() {
            output.push('\n');
        }
        if let Some(index) = separator_index(line)
            && is_sensitive_key(&line[..index].trim().to_ascii_lowercase())
        {
            append_redacted_tokens(&mut output, &line[..index + 1]);
            output.push_str(" [REDACTED]");
        } else {
            append_redacted_tokens(&mut output, line);
        }
    }
    if input.ends_with('\n') {
        output.push('\n');
    }
    output
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

fn append_redacted_tokens(output: &mut String, input: &str) {
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

#[cfg(test)]
mod compatibility_tests {
    use super::redact_secrets;

    // Preserve the old implementation as an independent compatibility oracle:
    // its line normalization and token/assignment ordering are observable.
    fn previous(input: &str) -> String {
        let mut output = String::with_capacity(input.len());
        for line in input.lines() {
            if !output.is_empty() {
                output.push('\n');
            }
            let index = [line.find(':'), line.find('=')].into_iter().flatten().min();
            let assigned = if let Some(index) = index {
                let key = line[..index].trim().to_ascii_lowercase();
                if [
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
                .any(|s| key.contains(s))
                {
                    format!("{} [REDACTED]", &line[..index + 1])
                } else {
                    line.to_owned()
                }
            } else {
                line.to_owned()
            };
            let mut rest = assigned.as_str();
            while let Some(index) = rest.find("sk-") {
                output.push_str(&rest[..index]);
                let candidate = &rest[index..];
                let end = candidate
                    .find(char::is_whitespace)
                    .unwrap_or(candidate.len());
                let token = &candidate[..end];
                output.push_str(if token.len() >= 12 {
                    "[REDACTED]"
                } else {
                    token
                });
                rest = &candidate[end..];
            }
            output.push_str(rest);
        }
        if input.ends_with('\n') {
            output.push('\n');
        }
        output
    }

    #[test]
    fn preserves_secret_matching_and_line_normalization() {
        let lines = [
            "",
            "ordinary",
            "API_KEY: value",
            "Password = value",
            "notsecret:x",
            "x=secret:x",
            "sk-12345678",
            "sk-123456789",
            "sk-123456789PASSWORD: value",
            "sk-12TOKEN=x",
            "스키마: sk-123456789",
            "normal=sk-123456789\u{2003}visible",
            "SK-123456789 sk-123456789",
            "token: sk-123456789",
            "x\rtoken:y",
        ];
        for prefix in ["", "\n", "\n\n", "\r\n"] {
            for separator in ["\n", "\r\n", "\r", "\u{2028}"] {
                for suffix in ["", "\n", "\n\n", "\r\n"] {
                    let input = format!("{prefix}{}{suffix}", lines.join(separator));
                    assert_eq!(redact_secrets(&input), previous(&input), "{input:?}");
                }
            }
        }
        assert_eq!(redact_secrets("\n\n"), "\n");
        assert_eq!(
            redact_secrets("sk-123456789PASSWORD: value"),
            "[REDACTED] [REDACTED]"
        );
    }
}
