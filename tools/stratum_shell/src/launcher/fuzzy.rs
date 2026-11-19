//! Fuzzy matching for the launcher: the query's characters must appear in
//! order in the candidate, and a match scores higher the more of it falls at
//! the start, at word starts, and in runs. Case is ignored.

/// The score of `query` against `candidate`, or `None` if it does not match.
/// An empty query matches everything with 0.
pub fn score(query: &str, candidate: &str) -> Option<i32> {
    let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    if query.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = candidate.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();

    // A whole-word or prefix match beats any scattered one.
    let joined: String = lower.iter().collect();
    let needle: String = query.iter().collect();
    let mut bonus = 0;
    if joined == needle {
        bonus += 1000;
    } else if joined.starts_with(&needle) {
        bonus += 400;
    } else if word_starts(&chars).iter().any(|&i| joined[byte(&lower, i)..].starts_with(&needle)) {
        bonus += 250;
    } else if joined.contains(&needle) {
        bonus += 120;
    }

    // Greedy in-order match, preferring word starts: take the next word
    // start holding the character if there is one before the next plain
    // occurrence's run would end.
    let starts = word_starts(&chars);
    let mut score = 0;
    let mut position = 0;
    let mut previous: Option<usize> = None;
    let mut all_starts = true;
    for &wanted in &query {
        let plain = (position..lower.len()).find(|&i| lower[i] == wanted)?;
        let at_start = starts.iter().copied().find(|&i| i >= position && lower[i] == wanted);
        // Continue a run if the plain match does; otherwise jump to a word
        // start when there is one.
        let index = match (previous, at_start) {
            (Some(p), _) if plain == p + 1 => plain,
            (_, Some(start)) => start,
            _ => plain,
        };
        score += 10;
        if starts.contains(&index) {
            score += 30;
        } else {
            all_starts = false;
        }
        if index == 0 {
            score += 20;
        }
        match previous {
            Some(p) if index == p + 1 => score += 25,
            Some(p) => score -= (index - p - 1).min(10) as i32,
            None => score -= index.min(15) as i32,
        }
        previous = Some(index);
        position = index + 1;
    }
    // An acronym ("vsc" for Visual Studio Code) beats a run that happens to
    // share letters.
    if all_starts && query.len() > 1 {
        score += 40;
    }
    // Shorter candidates win ties: "Files" over "Files and Folders Backup".
    Some(score + bonus - (chars.len() as i32 / 4))
}

/// Indices that begin a word: the first character, and any letter or digit
/// after a separator or at a lower-to-upper case change.
fn word_starts(chars: &[char]) -> Vec<usize> {
    (0..chars.len())
        .filter(|&i| {
            let c = chars[i];
            if !c.is_alphanumeric() {
                return false;
            }
            match i.checked_sub(1).map(|p| chars[p]) {
                None => true,
                Some(p) => !p.is_alphanumeric() || (p.is_lowercase() && c.is_uppercase()),
            }
        })
        .collect()
}

/// Byte offset of char index `i` in the string made of `chars`.
fn byte(chars: &[char], i: usize) -> usize {
    chars[..i].iter().map(|c| c.len_utf8()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn best<'a>(query: &str, candidates: &[&'a str]) -> Vec<&'a str> {
        let mut scored: Vec<_> = candidates.iter().filter_map(|c| score(query, c).map(|s| (s, *c))).collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.into_iter().map(|(_, c)| c).collect()
    }

    #[test]
    fn characters_must_appear_in_order() {
        assert!(score("frx", "Firefox").is_some());
        assert!(score("xof", "Firefox").is_none());
        assert!(score("zz", "Firefox").is_none());
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn prefixes_and_word_starts_beat_scattered_matches() {
        assert_eq!(best("fire", &["Wireshark Interface", "Firefox"])[0], "Firefox");
        assert_eq!(best("code", &["Barcode Reader", "Visual Studio Code", "Code"])[0], "Code");
        assert_eq!(best("vsc", &["Visual Studio Code", "Volume Scaler"])[0], "Visual Studio Code");
        assert_eq!(best("term", &["Kitty Terminal", "Thermal Monitor"])[0], "Kitty Terminal");
    }

    #[test]
    fn camel_case_counts_as_word_starts() {
        assert_eq!(best("ob", &["OBS Studio", "Thunderbird"])[0], "OBS Studio");
        assert!(score("vc", "VSCodium").is_some());
    }

    #[test]
    fn case_and_spaces_in_the_query_are_ignored() {
        assert_eq!(score("FIRE fox", "firefox"), score("firefox", "Firefox"));
    }
}
