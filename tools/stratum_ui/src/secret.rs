//! A password buffer that is wiped from memory when dropped: the text
//! fields' contents, and what the lock screen and polkit hand to PAM.

/// A password that is wiped from memory when dropped.
///
/// The buffer is allocated with spare capacity so typing does not reallocate
/// and leave stale copies behind.
pub struct Secret(String);

impl Secret {
    pub fn new() -> Self {
        Self(String::with_capacity(256))
    }

    pub fn push(&mut self, ch: char) {
        // Refuse to grow past the reserved buffer rather than reallocate.
        if self.0.len() + ch.len_utf8() <= self.0.capacity() {
            self.0.push(ch);
        }
    }

    pub fn pop(&mut self) {
        if let Some(ch) = self.0.pop() {
            // `pop` leaves the removed bytes in the buffer's spare capacity.
            // SAFETY: only spare capacity is written, which the string's UTF-8
            // invariant does not cover.
            unsafe {
                let spare = self.0.as_mut_vec().spare_capacity_mut();
                for byte in spare.iter_mut().take(ch.len_utf8()) {
                    byte.write(0);
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.wipe();
    }

    /// Number of characters, for drawing one dot per character.
    pub fn chars(&self) -> usize {
        self.0.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Move the contents out, leaving this buffer empty.
    pub fn take(&mut self) -> Secret {
        let mut taken = Secret::new();
        for ch in self.0.chars() {
            taken.push(ch);
        }
        self.wipe();
        taken
    }

    /// The password itself, for handing to a checker (PAM, polkit's helper).
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn wipe(&mut self) {
        // SAFETY: zero bytes are valid UTF-8, and the length is reset to 0
        // straight after.
        unsafe {
            let bytes = self.0.as_mut_vec();
            bytes.iter_mut().for_each(|byte| *byte = 0);
            bytes.set_len(0);
        }
    }
}

impl Default for Secret {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(text: &str) -> Secret {
        let mut secret = Secret::new();
        for ch in text.chars() {
            secret.push(ch);
        }
        secret
    }

    #[test]
    fn secret_edits_like_a_text_field() {
        let mut value = secret("héllo");
        assert_eq!(value.chars(), 5);
        value.pop();
        value.pop();
        assert_eq!(value.as_str(), "hél");
        let taken = value.take();
        assert!(value.is_empty());
        assert_eq!(taken.as_str(), "hél");
    }

    #[test]
    fn secret_never_reallocates() {
        let mut value = Secret::new();
        let capacity = value.0.capacity();
        for _ in 0..10_000 {
            value.push('x');
        }
        assert_eq!(value.0.capacity(), capacity, "typing must not move the buffer");
    }
}
