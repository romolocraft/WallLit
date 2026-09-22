use crate::stats::Instant;

#[derive(Default)]
pub struct Retry {
    attempts: u32,
    due: Option<Instant>,
}

pub fn delay_100ns(attempt: u32) -> i64 {
    (1i64 << attempt.min(5)) * 10_000_000
}

impl Retry {
    pub fn request(&mut self) {
        if self.due.is_none() { self.due = Some(Instant::now()); }
    }
    pub fn failed(&mut self) {
        self.due = Some(Instant::after(delay_100ns(self.attempts)));
        self.attempts = self.attempts.saturating_add(1);
    }
    pub fn clear(&mut self) { *self = Self::default(); }
    pub fn deadline(&self) -> Option<i64> { self.due.map(|d| d.until_100ns()) }
    pub fn ready(&self) -> bool { self.deadline().is_some_and(|d| d <= 0) }
    pub fn pending(&self) -> bool { self.due.is_some() }
}

pub fn candidates(start: usize, length: usize) -> impl Iterator<Item = usize> {
    (0..length).map(move |offset| (start + offset) % length)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_is_bounded_and_playlist_visits_every_item_once() {
        assert_eq!(delay_100ns(0), 10_000_000);
        assert_eq!(delay_100ns(100), 320_000_000);
        assert_eq!(candidates(2, 3).collect::<Vec<_>>(), [2, 0, 1]);
        assert_eq!(candidates(0, 0).count(), 0);
    }
}
