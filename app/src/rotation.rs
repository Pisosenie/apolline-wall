use std::path::{Path, PathBuf};

use apolline_wall_protocol::protocol::{Command, Playlist};
use iced::Task;

use crate::{App, Message};

impl App {
    fn interval_secs(&self) -> u64 {
        u64::from(self.config.rotation_minutes.clamp(1, 60)) * 60
    }

    /// Rotates what the library shows now.
    pub fn rotate(&self) -> Task<Message> {
        let sources = self
            .visible()
            .into_iter()
            .filter(|item| item.what.missing().is_none())
            .map(|item| item.path.clone())
            .collect::<Vec<_>>();
        if sources.is_empty() {
            return Task::none();
        }
        self.ask(Command::Playlist(Playlist {
            output: self.target.clone(),
            settings: self.config.playlist_settings(&sources),
            sources,
            folder: None,
            interval_secs: self.interval_secs(),
            fit: self.fit,
        }))
    }

    /// Rotates a folder: the daemon reads it again at every step, so what's added joins.
    pub fn rotate_folder(&self, folder: PathBuf) -> Task<Message> {
        let known = self
            .items
            .iter()
            .filter(|item| item.path.parent() == Some(folder.as_path()))
            .map(|item| item.path.clone())
            .collect::<Vec<_>>();
        self.ask(Command::Playlist(Playlist {
            output: self.target.clone(),
            settings: self.config.playlist_settings(&known),
            sources: Vec::new(),
            folder: Some(folder),
            interval_secs: self.interval_secs(),
            fit: self.fit,
        }))
    }

    pub fn stop_playlist(&self, output: String) -> Task<Message> {
        self.ask(Command::Playlist(Playlist {
            output: Some(output),
            sources: Vec::new(),
            folder: None,
            settings: Default::default(),
            interval_secs: 300,
            fit: self.fit,
        }))
    }

    /// The folder a wallpaper of the user's own folders sits in, to rotate it whole.
    pub fn folder_of(&self, path: &Path) -> Option<PathBuf> {
        folder_of(path, &self.folders())
    }
}

fn folder_of(path: &Path, folders: &[PathBuf]) -> Option<PathBuf> {
    let parent = path.parent()?;
    folders
        .iter()
        .any(|folder| parent.starts_with(folder))
        .then(|| parent.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_wallpapers_of_the_library_folders_offer_theirs() {
        let folders = [PathBuf::from("/home/u/Pictures/Wallpapers")];
        assert_eq!(
            folder_of(Path::new("/home/u/Pictures/Wallpapers/a.jpg"), &folders),
            Some(PathBuf::from("/home/u/Pictures/Wallpapers"))
        );
        assert_eq!(
            folder_of(
                Path::new("/home/u/Pictures/Wallpapers/Night/b.mp4"),
                &folders
            ),
            Some(PathBuf::from("/home/u/Pictures/Wallpapers/Night"))
        );
        assert_eq!(
            folder_of(
                Path::new("/home/u/.steam/workshop/content/431960/1/"),
                &folders
            ),
            None
        );
        assert_eq!(
            folder_of(Path::new("/home/u/Pictures/WallpapersOld/c.jpg"), &folders),
            None
        );
    }
}
