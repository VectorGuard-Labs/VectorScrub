//! Which passwords are refused when one is chosen (registration and password change).
//!
//! Length (12 to 128 characters) is checked by the request validators. This adds a check for the
//! passwords people actually pick when only a length is demanded: a common word with digits or
//! symbols on the end, a keyboard walk, a counting sequence, one repeated character or pattern, or
//! something built from the person's own email address.
//!
//! What this is NOT: a lookup against a corpus of leaked passwords. That needs either a large
//! list shipped with the service or a call to a third party (such as the Pwned Passwords range
//! API), and a call to a third party is a change to what the Privacy Policy says VectorScrub
//! shares and with whom. It is deliberately not done; this check is local and sends nothing
//! anywhere. Do not describe it as a breached-password check.
//!
//! The message never says which rule failed beyond a short reason, and the password is never
//! logged or echoed.

/// Why a password was refused. Safe to show to the person choosing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    TooCommon,
    TooPredictable,
    ContainsEmail,
}

impl Rejection {
    pub fn message(self) -> &'static str {
        match self {
            Rejection::TooCommon => "That password is too common. Choose something less guessable, such as a few unrelated words.",
            Rejection::TooPredictable => "That password follows a predictable pattern (repeats, a sequence or a keyboard row). Choose something less guessable, such as a few unrelated words.",
            Rejection::ContainsEmail => "Your password may not contain your email address. Choose something less guessable, such as a few unrelated words.",
        }
    }
}

/// Words that, with digits or symbols on the end and common letter swaps, are the most guessed
/// passwords. Matched exactly (after the swaps), never as a substring, so a passphrase that merely
/// contains one of these words is fine.
const COMMON_WORDS: &[&str] = &[
    "password", "passw0rd", "passwort", "letmein", "welcome", "iloveyou", "administrator", "admin",
    "changeme", "football", "baseball", "basketball", "soccer", "hockey", "monkey", "dragon", "master",
    "sunshine", "princess", "trustno", "superman", "batman", "spiderman", "starwars", "whatever",
    "freedom", "shadow", "michael", "jennifer", "jessica", "ashley", "charlie", "mustang", "ninja",
    "qwerty", "qwertyuiop", "asdfghjkl", "zxcvbnm", "azerty", "abcdefgh", "abcdefghijkl", "login",
    "access", "flower", "hello", "hellothere", "secret", "secure", "security", "private", "computer",
    "internet", "samsung", "google", "facebook", "twitter", "youtube", "amazon", "apple", "iphone",
    "windows", "linux", "killer", "hunter", "soccer", "summer", "winter", "spring", "autumn", "january",
    "february", "september", "october", "november", "december", "monday", "friday", "sunday",
    "liverpool", "arsenal", "chelsea", "manchester", "yankees", "cowboys", "lakers", "pokemon",
    "minecraft", "fortnite", "mario", "zelda", "naruto", "guitar", "music", "love", "lovely", "angel",
    "heaven", "forever", "family", "friends", "friend", "mother", "father", "daddy", "mommy", "baby",
    "cookie", "chocolate", "banana", "cheese", "pepper", "ginger", "coffee", "tigger", "pooh",
    "buster", "rocket", "silver", "golden", "diamond", "matrix", "master", "mypassword", "thepassword",
    "yourpassword", "newpassword", "mypass", "mysecret", "vectorscrub", "vectorguard", "scrub",
    "iloveyou", "ihateyou", "loveyou", "blessed", "praise", "jesus", "america", "canada", "england",
];

/// Whole passwords (lowercase) that are on every list of common passwords or are plain patterns.
const COMMON_PASSWORDS: &[&str] = &[
    "123456789012", "1234567890123", "12345678901234", "123456789abc", "1q2w3e4r5t6y", "1q2w3e4r5t6y7u",
    "1qaz2wsx3edc", "1qazxsw23edc", "qazwsxedcrfv", "zaq12wsxcde3", "qwertyuiop[]", "qwertyuiop123",
    "asdfghjkl;'", "zxcvbnm,./", "abcdefghijkl", "abcdefghijklm", "aaaaaaaaaaaa", "000000000000",
    "111111111111", "999999999999", "123123123123", "121212121212", "112233445566", "123321123321",
    "letmeinletmein", "passwordpassword", "iloveyouiloveyou", "p@ssw0rd1234", "p@ssword1234",
    "password@123", "password#123", "pass1234pass", "admin@123456", "welcome@12345",
];

/// Rows of a keyboard and runs of digits, used to spot a walk along them.
const WALKS: &[&str] = &[
    "qwertyuiopasdfghjklzxcvbnm", "qazwsxedcrfvtgbyhnujmikolp", "1234567890", "abcdefghijklmnopqrstuvwxyz",
    "azertyuiopqsdfghjklmwxcvbn", "1qaz2wsx3edc4rfv5tgb", "1q2w3e4r5t6y7u8i9o0p",
];

fn is_trailing(c: char) -> bool {
    c.is_ascii_digit() || !c.is_alphanumeric()
}

/// Common letter swaps people make (a password with `@` for `a` is still that password).
fn unswap(c: char) -> char {
    match c {
        '0' => 'o',
        '1' => 'l',
        '3' => 'e',
        '4' | '@' => 'a',
        '5' | '$' => 's',
        '7' => 't',
        '!' => 'i',
        other => other,
    }
}

/// The password with trailing digits and symbols removed (at most 8 of them) and letter swaps
/// undone, e.g. `P@ssw0rd2024!` becomes `password`.
fn skeleton(lower: &str) -> String {
    let mut chars: Vec<char> = lower.chars().collect();
    let mut stripped = 0;
    while stripped < 8 && chars.last().is_some_and(|c| is_trailing(*c)) && chars.len() > 1 {
        chars.pop();
        stripped += 1;
    }
    chars.into_iter().map(unswap).collect()
}

fn is_repeating(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 6 {
        return false;
    }
    // A short block repeated to fill the password: "abababababab", "123123123123", "xyz9xyz9xyz9".
    (1..=chars.len() / 3).any(|block| chars.iter().enumerate().all(|(i, c)| *c == chars[i % block]))
}

fn is_sequence(s: &str) -> bool {
    let chars: Vec<u32> = s.chars().map(|c| c as u32).collect();
    if chars.len() < 8 {
        return false;
    }
    let steps = chars.windows(2).filter(|w| w[1] as i64 - w[0] as i64 == 1).count();
    let back = chars.windows(2).filter(|w| w[0] as i64 - w[1] as i64 == 1).count();
    let pairs = chars.len() - 1;
    // Mostly counting up or down ("123456789012", "abcdefghijkl", "9876543210zz").
    steps * 10 >= pairs * 8 || back * 10 >= pairs * 8
}

fn is_walk(s: &str) -> bool {
    s.chars().count() >= 8
        && WALKS.iter().any(|row| {
            let rev: String = row.chars().rev().collect();
            row.contains(s) || rev.contains(s)
        })
}

/// Check a password against the rules above. `email` is the account's address (case ignored).
pub fn check(password: &str, email: &str) -> Result<(), Rejection> {
    let lower = password.trim().to_lowercase();
    let email = email.trim().to_lowercase();

    if !email.is_empty() {
        let local = email.split('@').next().unwrap_or("");
        if lower.contains(&email) || (local.chars().count() >= 4 && lower.contains(local)) {
            return Err(Rejection::ContainsEmail);
        }
    }

    if COMMON_PASSWORDS.contains(&lower.as_str()) {
        return Err(Rejection::TooCommon);
    }
    let skel = skeleton(&lower);
    let swapped: String = lower.chars().map(unswap).collect();
    if COMMON_WORDS.iter().any(|w| skel == *w || swapped == *w) {
        return Err(Rejection::TooCommon);
    }
    // A common word twice ("passwordpassword1") is as weak as once.
    if COMMON_WORDS.iter().any(|w| w.len() >= 4 && skel.len() == 2 * w.len() && skel == format!("{w}{w}")) {
        return Err(Rejection::TooCommon);
    }

    let distinct: std::collections::HashSet<char> = lower.chars().collect();
    if distinct.len() < 5 || is_repeating(&lower) || is_sequence(&lower) || is_walk(&lower) || is_walk(&skel) {
        return Err(Rejection::TooPredictable);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMAIL: &str = "jane.smith@example.com";

    #[test]
    fn common_words_with_digits_symbols_and_swaps_are_refused() {
        for bad in [
            "password1234", "Password123!", "P@ssw0rd2024!", "Passw0rd!!!!!", "letmein12345", "Welcome12345!",
            "iloveyou1234", "Administrator1", "qwertyuiop12", "Football2024!!", "changeme12345", "Dragon123456",
            "passwordpassword", "PasswordPassword1", "vectorscrub123", "MyPassword12345",
        ] {
            assert_eq!(check(bad, EMAIL), Err(Rejection::TooCommon), "{bad}");
        }
    }

    #[test]
    fn patterns_are_refused() {
        for bad in [
            "123456789012", "abcdefghijkl", "aaaaaaaaaaaa", "abababababab", "123123123123", "987654321098",
            "qwertyuiopasdf", "asdfghjklzxcv", "1q2w3e4r5t6y", "000000000000", "mnbvcxzlkjhg",
        ] {
            assert!(check(bad, EMAIL).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_password_built_from_the_email_is_refused() {
        assert_eq!(check("jane.smith@example.com", EMAIL), Err(Rejection::ContainsEmail));
        assert_eq!(check("Jane.Smith-is-here-2024", EMAIL), Err(Rejection::ContainsEmail));
        assert_eq!(check("xx-JANE.SMITH-xx-yy", EMAIL), Err(Rejection::ContainsEmail));
        // A very short local part is not treated as a secret to avoid (it would match too much).
        assert!(check("correct horse battery staple", "ab@example.com").is_ok());
    }

    #[test]
    fn ordinary_strong_passwords_and_passphrases_are_accepted() {
        for good in [
            "correct horse battery 91",
            "Tr0ub4dor&3-and-a-llama",
            "purple-giraffe-eats-lunch",
            "k9#Vw2!pQz7$Lm4x",
            "the password is in the garden shed", // contains a common word, but it is a passphrase
            "my dog Rex loves the beach 2019",
            "Blue.Harbour.Lantern.Quiet.77",
        ] {
            assert_eq!(check(good, EMAIL), Ok(()), "{good}");
        }
    }

    #[test]
    fn the_rejection_texts_never_echo_the_password() {
        for r in [Rejection::TooCommon, Rejection::TooPredictable, Rejection::ContainsEmail] {
            assert!(r.message().len() > 20 && !r.message().contains("password1234"));
        }
    }
}
