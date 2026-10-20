/// Milliseconds as `M:SS`, truncating partial seconds.
pub fn fmt_ms(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_minutes_and_seconds() {
        assert_eq!(fmt_ms(0), "0:00");
        assert_eq!(fmt_ms(4_999), "0:04");
        assert_eq!(fmt_ms(65_000), "1:05");
        assert_eq!(fmt_ms(600_000), "10:00");
    }
}
