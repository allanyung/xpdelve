use std::fmt::Write;

/// Make untrusted text inert before rendering it in a terminal.
pub fn sanitize(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '\n' | '\t' => output.push(character),
            '\r' => output.push_str("\\r"),
            '\u{1b}' => output.push_str("\\x1b"),
            '\u{07}' => output.push_str("\\x07"),
            character if character.is_control() || is_bidi_control(character) => {
                let _ = write!(output, "\\u{{{:04X}}}", u32::from(character));
            }
            character => output.push(character),
        }
    }
    output
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_terminal_and_bidi_controls_but_keeps_layout() {
        let input = "ok\n\t\x1b]52;c;owned\x07\r\u{202e}txt";
        let sanitized = sanitize(input);
        assert_eq!(sanitized, "ok\n\t\\x1b]52;c;owned\\x07\\r\\u{202E}txt");
        assert!(!sanitized.contains('\x1b'));
        assert!(!sanitized.contains('\x07'));
        assert!(!sanitized.contains('\r'));
        assert!(!sanitized.contains('\u{202e}'));
    }
}
