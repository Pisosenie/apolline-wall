use std::collections::BTreeMap;

#[derive(Default)]
pub struct Pending(BTreeMap<String, u64>);

impl Pending {
    pub fn wait(&mut self, output: String, generation: u64) {
        self.0.insert(output, generation);
    }

    pub fn take(&mut self, mut current: impl FnMut(&str, u64) -> bool) -> Vec<String> {
        std::mem::take(&mut self.0)
            .into_iter()
            .filter_map(|(output, generation)| current(&output, generation).then_some(output))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delayed_poster_only_announces_monitors_still_showing_that_generation() {
        let mut pending = Pending::default();
        pending.wait("DP-1".into(), 1);
        pending.wait("DP-2".into(), 2);
        assert_eq!(
            pending.take(|output, generation| output == "DP-2" && generation == 2),
            ["DP-2"]
        );
        assert!(pending.take(|_, _| true).is_empty());
    }

    #[test]
    fn repeated_attachment_replaces_the_old_ticket_instead_of_duplicating_ready() {
        let mut pending = Pending::default();
        pending.wait("DP-1".into(), 1);
        pending.wait("DP-1".into(), 3);
        assert_eq!(pending.take(|_, generation| generation == 3), ["DP-1"]);
        pending.wait("DP-1".into(), 4);
        assert!(pending.take(|_, generation| generation == 3).is_empty());
    }
}
