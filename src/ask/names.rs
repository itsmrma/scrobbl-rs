//! Unicode-safe name normalization used across projection edition keys and
//! public image matching. Stored names and UI labels are never rewritten.

/// Unicode-safe, dependency-free fold used only for matching; stored names and
/// the values shown to the user are never rewritten.
pub fn fold(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for character in input.chars() {
        if ('\u{0300}'..='\u{036f}').contains(&character) {
            continue;
        }
        for lower in character.to_lowercase() {
            match lower {
                'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => out.push('a'),
                'ç' | 'ć' | 'č' => out.push('c'),
                'ď' | 'đ' => out.push('d'),
                'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' => out.push('e'),
                'ğ' => out.push('g'),
                'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' => out.push('i'),
                'ł' => out.push('l'),
                'ñ' | 'ń' => out.push('n'),
                'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => out.push('o'),
                'ř' => out.push('r'),
                'ś' | 'š' => out.push('s'),
                'ť' => out.push('t'),
                'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => out.push('u'),
                'ý' | 'ÿ' => out.push('y'),
                'ź' | 'ż' => out.push('z'),
                'æ' => out.push_str("ae"),
                'œ' => out.push_str("oe"),
                'ß' => out.push_str("ss"),
                '’' | '‘' | '`' => out.push('\''),
                '–' | '—' => out.push('-'),
                other => out.push(other),
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::fold;

    #[test]
    fn fold_is_case_and_accent_insensitive_without_dropping_symbols() {
        assert_eq!(fold("Björk"), fold("BJORK"));
        assert_eq!(fold("Beyoncé"), "beyonce");
        assert_eq!(fold("AC/DC"), "ac/dc");
        assert_eq!(fold("Guns N’ Roses"), "guns n' roses");
        assert_eq!(fold("坂本龍一"), "坂本龍一");
        assert_eq!(fold("  Sigur   Rós "), "sigur ros");
        assert_eq!(fold("Sgt. Pepper's"), "sgt. pepper's");
        assert_eq!(fold("Straße"), "strasse");
    }
}
