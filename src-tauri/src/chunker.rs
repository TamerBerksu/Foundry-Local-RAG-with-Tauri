use std::collections::HashMap;

pub struct Parsed {
    pub meta: HashMap<String, String>,
    pub body: String,
}

pub fn parse_front_matter(text: &str) -> Parsed {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let plain = || Parsed {
        meta: HashMap::new(),
        body: text.to_string(),
    };

    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return plain();
    };
    if first.trim_end() != "---" {
        return plain();
    }

    let mut consumed = first.len();
    let mut meta = HashMap::new();
    for line in lines {
        consumed += line.len();
        let line = line.trim_end();
        if line == "---" {
            return Parsed {
                meta,
                body: text[consumed..].to_string(),
            };
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            if !key.is_empty() {
                meta.insert(key.to_lowercase(), value.trim().to_string());
            }
        }
    }
    plain()
}

pub fn chunk_text(text: &str, size: usize, overlap: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    if words.len() <= size {
        return vec![text.trim().to_string()];
    }

    let step = size.saturating_sub(overlap).max(1);
    let mut chunks = Vec::new();
    let mut start = 0;
    loop {
        let end = (start + size).min(words.len());
        chunks.push(words[start..end].join(" "));
        if end >= words.len() {
            break;
        }
        start += step;
    }
    chunks
}

pub fn term_frequency(text: &str) -> HashMap<String, f32> {
    let cleaned: String = text
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '\'' {
                c
            } else {
                ' '
            }
        })
        .collect();

    let mut tf = HashMap::new();
    for token in cleaned.split_whitespace() {
        let token = token.trim_matches(|c| c == '-' || c == '\'');
        if token.chars().count() > 1 {
            *tf.entry(token.to_string()).or_insert(0.0) += 1.0;
        }
    }
    tf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_is_split_from_body() {
        let parsed = parse_front_matter("---\ntitle: Moso\nid: A-1\n---\nbody text");
        assert_eq!(parsed.meta.get("title").map(String::as_str), Some("Moso"));
        assert_eq!(parsed.meta.get("id").map(String::as_str), Some("A-1"));
        assert_eq!(parsed.body, "body text");
    }

    #[test]
    fn text_without_front_matter_is_untouched() {
        let parsed = parse_front_matter("just words");
        assert!(parsed.meta.is_empty());
        assert_eq!(parsed.body, "just words");
    }

    #[test]
    fn windows_overlap() {
        let text: Vec<String> = (0..450).map(|i| format!("w{i}")).collect();
        let chunks = chunk_text(&text.join(" "), 200, 25);
        assert_eq!(chunks.len(), 3);
        assert!(chunks[1].starts_with("w175 "));
        assert!(chunks[2].ends_with("w449"));
    }

    #[test]
    fn short_text_is_one_chunk() {
        assert_eq!(chunk_text("  a b c  ", 200, 25), vec!["a b c".to_string()]);
        assert!(chunk_text("   ", 200, 25).is_empty());
    }

    #[test]
    fn terms_are_counted_across_scripts() {
        let tf = term_frequency("Bambu, bambu! Rhizome's rhizome's a");
        assert_eq!(tf.get("bambu"), Some(&2.0));
        assert_eq!(tf.get("rhizome's"), Some(&2.0));
        assert!(!tf.contains_key("a"));
        let tr = term_frequency("Şeker kamışı ŞEKER");
        assert_eq!(tr.get("şeker"), Some(&2.0));
    }
}
