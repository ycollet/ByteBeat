use std::{fmt, io::Write};

pub fn init() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|buffer, record| {
            let level_style = buffer.default_level_style(record.level());
            writeln!(
                buffer,
                "[{} {level_style}{:<5}{level_style:#} {}] {}",
                buffer.timestamp_seconds(),
                record.level(),
                TerminalText(record.target()),
                TerminalText(record.args()),
            )
        })
        .init();
}

struct TerminalText<T>(T);

impl<T: fmt::Display> fmt::Display for TerminalText<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::write(&mut EscapingWriter(formatter), format_args!("{}", self.0))
    }
}

struct EscapingWriter<'a, 'b>(&'a mut fmt::Formatter<'b>);

impl fmt::Write for EscapingWriter<'_, '_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let mut start = 0;
        for (index, character) in text.char_indices() {
            if character.is_control() && character != '\n' {
                self.0.write_str(&text[start..index])?;
                fmt::write(self.0, format_args!("{}", character.escape_default()))?;
                start = index + character.len_utf8();
            }
        }
        self.0.write_str(&text[start..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_readable_unicode_and_line_breaks() {
        let text = "first line\ncaf\u{e9} \u{1f3b5}\nlast line";
        assert_eq!(TerminalText(text).to_string(), text);
    }

    #[test]
    fn renders_terminal_commands_and_cursor_controls_visibly() {
        let text = "\x1b]52;c;QQ==\x07\x1b[2J\r\x08\t\0\x7f\u{9b}\u{9d}";
        assert_eq!(
            TerminalText(text).to_string(),
            r"\u{1b}]52;c;QQ==\u{7}\u{1b}[2J\r\u{8}\t\u{0}\u{7f}\u{9b}\u{9d}",
        );
    }

    #[test]
    fn only_line_feeds_survive_from_the_c0_and_c1_control_ranges() {
        for code in (0_u8..=31).chain(127..=159) {
            let character = char::from(code);
            let text = TerminalText(character).to_string();
            if character == '\n' {
                assert_eq!(text, "\n");
            } else {
                assert!(!text.chars().any(char::is_control), "{code}: {text:?}");
                assert_eq!(text, character.escape_default().to_string());
            }
        }
    }

    #[test]
    fn escapes_display_arguments_even_when_written_in_fragments() {
        struct Fragmented;
        impl fmt::Display for Fragmented {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("caf\u{e9} \x1b")?;
                formatter.write_str("]52;c;QQ==")?;
                formatter.write_str("\x07\nsecond line")
            }
        }
        assert_eq!(
            TerminalText(format_args!("{}: {}", 42, Fragmented)).to_string(),
            "42: caf\u{e9} \\u{1b}]52;c;QQ==\\u{7}\nsecond line",
        );
    }
}
