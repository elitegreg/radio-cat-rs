use std::time::Duration;

pub(crate) fn estimate_send_time(text: &str, wpm: u8) -> Option<Duration> {
    cw_serial_keyer::estimate_send_time(text, wpm).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_known_wpm() {
        assert!(estimate_send_time("E", 20).is_some());
    }

    #[test]
    fn rejects_unestimatable_wpm() {
        assert!(estimate_send_time("E", 4).is_none());
    }
}
