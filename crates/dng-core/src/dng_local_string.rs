// Port of dng_local_string.h/.cpp — a default string plus per-language
// translations.

use crate::dng_string::DngString;
use crate::dng_types::uint32;

#[derive(Debug, Clone, Default, PartialEq)]
struct DictionaryEntry {
    language: DngString,
    translation: DngString,
}

/// Port of `dng_local_string`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DngLocalString {
    default_text: DngString,
    dictionary: Vec<DictionaryEntry>,
}

impl DngLocalString {
    pub fn new() -> Self {
        DngLocalString::default()
    }

    /// `dng_local_string (const dng_string &s)`.
    pub fn from_string(s: &DngString) -> Self {
        DngLocalString {
            default_text: s.clone(),
            dictionary: Vec::new(),
        }
    }

    /// `Clear ()`.
    pub fn clear(&mut self) {
        self.default_text.clear();
        self.dictionary.clear();
    }

    /// `SetDefaultText (s)`.
    pub fn set_default_text(&mut self, s: &DngString) {
        self.default_text = s.clone();
    }

    /// `AddTranslation (language, translation)` — language truncated to 255
    /// bytes as in C++.
    pub fn add_translation(&mut self, language: &DngString, translation: &DngString) {
        let mut safe_language = language.clone();
        safe_language.truncate(255);
        self.dictionary.push(DictionaryEntry {
            language: safe_language,
            translation: translation.clone(),
        });
    }

    /// `Set (const char *s)`.
    pub fn set(&mut self, s: &[u8]) {
        let mut default_text = DngString::new();
        default_text.set(s);
        *self = DngLocalString::from_string(&default_text);
    }

    /// `DefaultText ()`.
    pub fn default_text(&self) -> &DngString {
        &self.default_text
    }

    pub fn default_text_mut(&mut self) -> &mut DngString {
        &mut self.default_text
    }

    /// `TranslationCount ()`.
    pub fn translation_count(&self) -> uint32 {
        self.dictionary.len() as uint32
    }

    /// `Language (index)`.
    pub fn language(&self, index: uint32) -> &DngString {
        &self.dictionary[index as usize].language
    }

    /// `Translation (index)`.
    pub fn translation(&self, index: uint32) -> &DngString {
        &self.dictionary[index as usize].translation
    }

    /// `LocalText (locale)`.
    pub fn local_text(&self, locale: &DngString) -> &DngString {
        // Pass 1 - try for a match starting with the entire locale string.
        if locale.length() >= 5 {
            for index in 0..self.translation_count() {
                if self.language(index).starts_with(locale.get(), false) {
                    return self.translation(index);
                }
            }
        }

        // Pass 2 - try for a language only match.
        if locale.length() >= 2 {
            let mut language_only = locale.clone();
            language_only.truncate(2);
            for index in 0..self.translation_count() {
                if self.language(index).starts_with(language_only.get(), false) {
                    return self.translation(index);
                }
            }
        }

        // Otherwise use default text.
        &self.default_text
    }

    /// `IsEmpty ()`.
    pub fn is_empty(&self) -> bool {
        self.default_text.is_empty()
    }

    /// `NotEmpty ()`.
    pub fn not_empty(&self) -> bool {
        !self.is_empty()
    }

    /// `Truncate (maxBytes)`.
    pub fn truncate(&mut self, max_bytes: uint32) {
        self.default_text.truncate(max_bytes);
        for entry in &mut self.dictionary {
            entry.translation.truncate(max_bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_text_matching() {
        let mut s = DngLocalString::new();
        let mut d = DngString::new();
        d.set(b"Hello");
        s.set_default_text(&d);

        let mut en = DngString::new();
        en.set(b"en-US");
        let mut de = DngString::new();
        de.set(b"de-DE");
        let mut hallo = DngString::new();
        hallo.set(b"Hallo");
        s.add_translation(&en, &DngString::from_bytes(b"Hello EN"));
        s.add_translation(&de, &hallo);

        // Full locale match.
        assert_eq!(s.local_text(&de).get(), b"Hallo");
        // Language-only match.
        assert_eq!(s.local_text(&DngString::from_bytes(b"de-AT")).get(), b"Hallo");
        // No match -> default.
        assert_eq!(s.local_text(&DngString::from_bytes(b"fr-FR")).get(), b"Hello");
        assert!(s.not_empty());
    }
}
