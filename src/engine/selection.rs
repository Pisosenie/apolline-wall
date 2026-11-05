use std::collections::{HashMap, HashSet};

use crate::protocol::{Adjust, Show};

#[derive(Default)]
pub(super) struct Selection {
    pub everywhere: Option<Show>,
    pub per_output: HashMap<String, Show>,
    pub inherited: HashSet<String>,
}

impl Selection {
    pub fn wanted(&self, output: &str) -> Option<Show> {
        self.per_output
            .get(output)
            .or(self.everywhere.as_ref())
            .cloned()
    }

    pub fn follows_shared(&self, output: &str) -> bool {
        !self.per_output.contains_key(output) || self.inherited.contains(output)
    }

    pub fn set(&mut self, show: Show) {
        match &show.output {
            None => {
                self.per_output.clear();
                self.inherited.clear();
                self.everywhere = Some(show);
            }
            Some(name) => {
                self.inherited.remove(name);
                self.per_output.insert(name.clone(), show);
            }
        }
    }

    pub fn rotate(&mut self, output: &Option<String>, show: Show) {
        if output.is_some() {
            self.set(show);
            return;
        }
        self.per_output.retain(|name, previous| {
            !self.inherited.contains(name) || previous.source == show.source
        });
        self.inherited
            .retain(|name| self.per_output.contains_key(name));
        self.everywhere = Some(show);
    }

    pub fn adjust(&mut self, adjust: &Adjust) {
        let look = Some(adjust.look).filter(|look| !look.is_plain());
        match &adjust.output {
            None => {
                for show in self
                    .everywhere
                    .iter_mut()
                    .chain(self.per_output.values_mut())
                {
                    show.look = look;
                }
                self.per_output
                    .retain(|name, _| !self.inherited.contains(name));
                self.inherited.clear();
            }
            Some(name) => {
                if let Some(show) = self.per_output.get_mut(name) {
                    show.look = look;
                } else if let Some(default) = &self.everywhere {
                    self.per_output.insert(
                        name.clone(),
                        Show {
                            output: Some(name.clone()),
                            look,
                            ..default.clone()
                        },
                    );
                    self.inherited.insert(name.clone());
                }
                if self.inherited.contains(name)
                    && self
                        .per_output
                        .get(name)
                        .zip(self.everywhere.as_ref())
                        .is_some_and(|(show, default)| {
                            show.look.unwrap_or_default() == default.look.unwrap_or_default()
                        })
                {
                    self.per_output.remove(name);
                    self.inherited.remove(name);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Fit, Look};

    fn show(output: Option<&str>, source: &str) -> Show {
        Show {
            output: output.map(Into::into),
            source: source.into(),
            properties: Default::default(),
            fit: Fit::Cover,
            transition_ms: 0,
            look: None,
        }
    }

    fn adjust(output: Option<&str>, look: Look) -> Adjust {
        Adjust {
            output: output.map(Into::into),
            look,
            draft: true,
        }
    }

    #[test]
    fn editing_one_monitor_does_not_detach_it_or_change_its_neighbour() {
        let mut selection = Selection::default();
        selection.set(show(None, "/a"));
        let dim = Look {
            brightness: -0.3,
            ..Default::default()
        };
        selection.adjust(&adjust(Some("DP-1"), dim));
        assert!(selection.follows_shared("DP-1"));
        assert_eq!(selection.wanted("DP-1").unwrap().look, Some(dim));
        assert_eq!(selection.wanted("DP-2").unwrap().look, None);
        selection.adjust(&adjust(Some("DP-1"), Look::default()));
        assert!(selection.per_output.is_empty());
        assert!(selection.inherited.is_empty());
    }

    #[test]
    fn rotation_replaces_inherited_edits_but_preserves_manual_assignments() {
        let mut selection = Selection::default();
        selection.set(show(None, "/a"));
        selection.adjust(&adjust(
            Some("DP-1"),
            Look {
                speed: 0.5,
                ..Default::default()
            },
        ));
        selection.set(show(Some("DP-2"), "/manual"));
        selection.rotate(&None, show(None, "/b"));
        assert_eq!(
            selection.wanted("DP-1").unwrap().source.to_str(),
            Some("/b")
        );
        assert_eq!(selection.wanted("DP-1").unwrap().look, None);
        assert_eq!(
            selection.wanted("DP-2").unwrap().source.to_str(),
            Some("/manual")
        );
        assert!(selection.follows_shared("DP-1"));
        assert!(!selection.follows_shared("DP-2"));
    }

    #[test]
    fn manual_choice_of_the_same_file_still_detaches_the_monitor() {
        let mut selection = Selection::default();
        selection.set(show(None, "/a"));
        selection.adjust(&adjust(
            Some("DP-1"),
            Look {
                speed: 0.5,
                ..Default::default()
            },
        ));
        selection.set(show(Some("DP-1"), "/a"));
        assert!(!selection.follows_shared("DP-1"));
        selection.rotate(&None, show(None, "/b"));
        assert_eq!(
            selection.wanted("DP-1").unwrap().source.to_str(),
            Some("/a")
        );
    }
}
