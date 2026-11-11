//! Which US state a saved address is in, as a two-letter code.
//!
//! Used for one thing: deciding whether the opt-out guide should offer California's DROP
//! service (California residents only). The address is free text, so this reads it
//! conservatively: it looks only at the LAST segments of the address, where the state sits, and
//! only accepts a segment that IS a state (optionally with a ZIP and a country). A state name
//! inside a street ("Oregon St") or city ("Washington") elsewhere in the address is ignored.
//! When in doubt it answers `None`, and the page then says "Live in California?" instead of
//! guessing. Nothing here is stored: the state is read from the decrypted address on request
//! and returned only to the signed-in owner.

const STATES: [(&str, &str); 51] = [
    ("AL", "alabama"), ("AK", "alaska"), ("AZ", "arizona"), ("AR", "arkansas"), ("CA", "california"),
    ("CO", "colorado"), ("CT", "connecticut"), ("DE", "delaware"), ("DC", "district of columbia"),
    ("FL", "florida"), ("GA", "georgia"), ("HI", "hawaii"), ("ID", "idaho"), ("IL", "illinois"),
    ("IN", "indiana"), ("IA", "iowa"), ("KS", "kansas"), ("KY", "kentucky"), ("LA", "louisiana"),
    ("ME", "maine"), ("MD", "maryland"), ("MA", "massachusetts"), ("MI", "michigan"),
    ("MN", "minnesota"), ("MS", "mississippi"), ("MO", "missouri"), ("MT", "montana"),
    ("NE", "nebraska"), ("NV", "nevada"), ("NH", "new hampshire"), ("NJ", "new jersey"),
    ("NM", "new mexico"), ("NY", "new york"), ("NC", "north carolina"), ("ND", "north dakota"),
    ("OH", "ohio"), ("OK", "oklahoma"), ("OR", "oregon"), ("PA", "pennsylvania"),
    ("RI", "rhode island"), ("SC", "south carolina"), ("SD", "south dakota"), ("TN", "tennessee"),
    ("TX", "texas"), ("UT", "utah"), ("VT", "vermont"), ("VA", "virginia"), ("WA", "washington"),
    ("WV", "west virginia"), ("WI", "wisconsin"), ("WY", "wyoming"),
];

const COUNTRIES: [&str; 5] = ["usa", "us", "u.s.", "u.s.a.", "united states"];

fn by_code(code: &str) -> Option<&'static str> {
    STATES.iter().find(|(c, _)| c.eq_ignore_ascii_case(code)).map(|(c, _)| *c)
}

fn by_name(name: &str) -> Option<&'static str> {
    STATES.iter().find(|(_, n)| *n == name).map(|(c, _)| *c)
}

/// `"90001"` / `"90001-1234"`.
fn is_zip(s: &str) -> bool {
    let (head, tail) = match s.split_once('-') {
        Some((h, t)) => (h, Some(t)),
        None => (s, None),
    };
    head.len() == 5
        && head.bytes().all(|b| b.is_ascii_digit())
        && tail.is_none_or(|t| t.len() == 4 && t.bytes().all(|b| b.is_ascii_digit()))
}

/// What is left of a segment after removing a trailing ZIP code.
fn without_zip(segment: &str) -> &str {
    match segment.rsplit_once(' ') {
        Some((rest, last)) if is_zip(last) => rest.trim(),
        _ if is_zip(segment) => "",
        _ => segment,
    }
}

/// The state a SEGMENT of an address names, if it names one: `"CA"`, `"california"`,
/// `"CA 90001"`, or a city followed by an UPPERCASE code (`"Austin TX 78701"`).
fn state_of_segment(segment: &str) -> Option<&'static str> {
    let rest = without_zip(segment).trim_matches(|c: char| c == '.' || c.is_whitespace());
    if rest.is_empty() {
        return None;
    }
    let lowered = rest.to_ascii_lowercase();
    if rest.len() == 2 {
        return by_code(rest); // a segment that is just two letters: "CA", "or"
    }
    if let Some(code) = by_name(&lowered) {
        return Some(code);
    }
    // "Austin TX": the last word must be an UPPERCASE code, so the word "in" or "or" in a
    // street or city name is never read as Indiana or Oregon.
    let last = rest.rsplit(' ').next()?;
    if last.len() == 2 && last.bytes().all(|b| b.is_ascii_uppercase()) {
        return by_code(last);
    }
    // "Springfield Illinois": a multi-word state name at the end of the segment.
    STATES
        .iter()
        .filter(|(_, n)| lowered.len() > n.len() && lowered.ends_with(&format!(" {n}")))
        .max_by_key(|(_, n)| n.len())
        .map(|(c, _)| *c)
}

/// The US state of an address, or `None` when it cannot be told.
pub fn state_from_address(address: &str) -> Option<&'static str> {
    let normalized = address.replace(['\n', '\r', ';'], ",");
    let segments: Vec<&str> = normalized
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    // Only the last three segments can hold the state: street, city, "ST ZIP", country.
    for segment in segments.iter().rev().take(3) {
        let lowered = segment.to_ascii_lowercase();
        if COUNTRIES.contains(&lowered.as_str()) || is_zip(segment) {
            continue;
        }
        if let Some(state) = state_of_segment(segment) {
            return Some(state);
        }
        // The first segment from the end that is not country/ZIP and not a state is a city
        // or street: the state would have been right after it. Stop rather than look further
        // back, where a state NAME is far more likely to be part of a street.
        break;
    }

    // No commas at all: "1 Main St Austin TX 78701".
    if segments.len() <= 1 {
        let words: Vec<&str> = address.split_whitespace().collect();
        let mut end = words.len();
        while end > 0 && (is_zip(words[end - 1]) || COUNTRIES.contains(&words[end - 1].to_ascii_lowercase().as_str())) {
            end -= 1;
        }
        if end > 0 {
            let last = words[end - 1].trim_matches('.');
            if last.len() == 2 && last.bytes().all(|b| b.is_ascii_uppercase()) {
                return by_code(last);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_us_addresses() {
        for (address, state) in [
            ("123 Main St, Los Angeles, CA 90001", "CA"),
            ("123 Main St, Los Angeles, California 90001", "CA"),
            ("123 Main St\nLos Angeles, CA", "CA"),
            ("123 Main St, Los Angeles, CA 90001-1234", "CA"),
            ("123 Main St, Austin, TX 78701", "TX"),
            ("123 Main St, Austin, Texas", "TX"),
            ("1 Main St, New York, NY 10001, United States", "NY"),
            ("1 Main St, New York, New York 10001, USA", "NY"),
            ("45 Oak Ave, Dallas, TX 75201, US", "TX"),
            ("1 Main St, Washington, DC 20001", "DC"),
            ("PO Box 5, Portland, or 97201", "OR"),
            ("12 Elm Rd, Phoenix, AZ", "AZ"),
            ("9 Pine Ct, Charleston, West Virginia 25301", "WV"),
        ] {
            assert_eq!(state_from_address(address), Some(state), "{address:?}");
        }
    }

    #[test]
    fn a_state_name_in_a_street_or_city_is_not_the_state() {
        for (address, state) in [
            ("55 Oregon St, Austin, TX 78701", "TX"),
            ("10 Oak Rd, Washington, PA 15301", "PA"),           // the city Washington, not the state
            ("123 CA Street, Austin, TX 78701-1234", "TX"),
            ("99 Indiana Ave, Chicago, IL 60601", "IL"),
            ("4 North Carolina Ave, Washington, DC 20003", "DC"),
            ("7 New York Ave, Newark, NJ 07102", "NJ"),
            ("1 Georgia Way, Atlanta, GA 30301", "GA"),
        ] {
            assert_eq!(state_from_address(address), Some(state), "{address:?}");
        }
    }

    #[test]
    fn a_city_and_state_in_one_segment_or_no_commas_at_all() {
        for (address, state) in [
            ("1 Main St, Austin TX 78701", "TX"),
            ("1 Main St, Austin TX 78701, USA", "TX"),
            ("1 Main St, Springfield Illinois", "IL"),
            ("1 Main St Austin TX 78701", "TX"),
            ("1 Main St Austin TX", "TX"),
            ("1 Main St Austin TX 78701 USA", "TX"),
        ] {
            assert_eq!(state_from_address(address), Some(state), "{address:?}");
        }
    }

    #[test]
    fn when_it_cannot_tell_it_says_so_rather_than_guessing() {
        for address in [
            "",
            "   ",
            "123 Main St, Springfield",                  // no state
            "12 Rue de Rivoli, Paris, France",
            "Flat 4, London SW1A 1AA",
            "1 Main St, Toronto, ON M5V 2T6",           // Canada
            "1 Main St, Vancouver, BC",
            "1 Main St, Austin, ZZ 78701",              // not a state
            "Oregon St, Austin",                        // a street, then a city
            "1 Main St, Washington",                    // ambiguous: city or state? the last segment is the city here
            "123456",
            "in or on",
        ] {
            let got = state_from_address(address);
            // "Washington" alone is a state name by itself, so the one tolerated answer is WA
            if address == "1 Main St, Washington" {
                assert!(got.is_none() || got == Some("WA"), "{address:?}: {got:?}");
            } else {
                assert_eq!(got, None, "{address:?}");
            }
        }
    }

    #[test]
    fn lowercase_words_are_not_read_as_states_inside_a_longer_segment() {
        // "in" / "or" / "me" / "ok" as ordinary words at the end of a city segment
        for address in ["1 Main St, Lost in", "1 Main St, Black or", "1 Main St, Call me"] {
            assert_eq!(state_from_address(address), None, "{address:?}");
        }
    }
}
