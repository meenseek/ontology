#[must_use]
pub fn expanded_search_text(parts: &[&str]) -> String {
    let mut tokens = Vec::new();
    for part in parts {
        tokens.extend(base_tokens(part));
    }

    let mut expanded = Vec::new();
    for token in tokens {
        push_unique(&mut expanded, token.clone());
        if should_expand_token(&token) {
            for gram in character_ngrams(&token) {
                push_unique(&mut expanded, gram);
            }
        }
    }
    expanded.join(" ")
}

#[must_use]
pub fn expanded_query_terms(query: &str) -> Vec<Vec<String>> {
    base_tokens(query)
        .into_iter()
        .map(|token| match_terms_for_token(&token))
        .filter(|terms| !terms.is_empty())
        .collect()
}

fn match_terms_for_token(token: &str) -> Vec<String> {
    let mut terms = Vec::new();
    push_unique(&mut terms, token.to_owned());

    if should_expand_token(token) {
        for gram in character_ngrams(token) {
            push_unique(&mut terms, gram);
        }
    }

    terms
}

fn base_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for character in input.chars().flat_map(char::to_lowercase) {
        if is_search_character(character) {
            current.push(character);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn is_search_character(character: char) -> bool {
    character.is_alphanumeric() || is_korean_syllable(character) || is_cjk_character(character)
}

fn should_expand_token(token: &str) -> bool {
    token.chars().any(is_cjk_or_korean) && token.chars().count() >= 2
}

fn character_ngrams(token: &str) -> Vec<String> {
    let characters = token.chars().collect::<Vec<_>>();
    let mut grams = Vec::new();

    for size in [2, 3] {
        if characters.len() < size {
            continue;
        }
        for window in characters.windows(size) {
            push_unique(&mut grams, window.iter().collect::<String>());
        }
    }

    grams
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !value.is_empty() && !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn is_cjk_or_korean(character: char) -> bool {
    is_korean_syllable(character) || is_cjk_character(character)
}

fn is_korean_syllable(character: char) -> bool {
    matches!(character, '\u{ac00}'..='\u{d7a3}')
}

fn is_cjk_character(character: char) -> bool {
    matches!(
        character,
        '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{f900}'..='\u{faff}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_korean_compound_tokens() {
        let expanded = expanded_search_text(&["코드리뷰 테스트누락"]);

        assert!(expanded.contains("코드리뷰"));
        assert!(expanded.contains("코드"));
        assert!(expanded.contains("리뷰"));
        assert!(expanded.contains("테스"));
        assert!(expanded.contains("스트누"));
    }

    #[test]
    fn builds_expanded_match_query_for_korean() {
        let query = expanded_query_terms("코드리뷰");

        assert_eq!(query.len(), 1);
        assert!(query[0].contains(&"코드리뷰".to_owned()));
        assert!(query[0].contains(&"코드".to_owned()));
    }

    #[test]
    fn keeps_ascii_queries_simple() {
        let query = expanded_query_terms("rust review");

        assert_eq!(query, vec![vec!["rust"], vec!["review"]]);
    }
}
