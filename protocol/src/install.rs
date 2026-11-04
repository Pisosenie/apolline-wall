//! What to install for a missing piece, said the way this distribution says it.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// libmpv, for videos.
    Mpv,
    /// For video thumbnails in the app.
    Ffmpeg,
    /// linux-wallpaperengine, for Wallpaper Engine's scenes and web wallpapers.
    SceneRenderer,
    /// For Workshop downloads without the Steam client.
    Steamcmd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Distro {
    Arch,
    Debian,
    Fedora,
    Suse,
    Other,
}

const SCENE_RENDERER: &str = "https://github.com/Almamu/linux-wallpaperengine";
const STEAMCMD: &str = "https://developer.valvesoftware.com/wiki/SteamCMD";

/// How to get it here, as a phrase to end a sentence with.
pub fn hint(need: Need) -> String {
    let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    hint_for(need, distro(&os_release))
}

fn hint_for(need: Need, distro: Distro) -> String {
    let package = |manager: &str, name: &str| format!("install {name} (`{manager} {name}`)");
    match (need, distro) {
        (Need::Mpv, Distro::Arch) => package("sudo pacman -S", "mpv"),
        (Need::Mpv, Distro::Debian) => package("sudo apt install", "libmpv2"),
        (Need::Mpv, Distro::Fedora) => {
            format!(
                "{} from RPM Fusion",
                package("sudo dnf install", "mpv-libs")
            )
        }
        (Need::Mpv, Distro::Suse) => package("sudo zypper install", "libmpv2"),
        (Need::Mpv, Distro::Other) => "install libmpv, which comes with mpv".into(),
        (Need::Ffmpeg, Distro::Arch) => package("sudo pacman -S", "ffmpeg"),
        (Need::Ffmpeg, Distro::Debian) => package("sudo apt install", "ffmpeg"),
        (Need::Ffmpeg, Distro::Fedora) => package("sudo dnf install", "ffmpeg-free"),
        (Need::Ffmpeg, Distro::Suse) => package("sudo zypper install", "ffmpeg"),
        (Need::Ffmpeg, Distro::Other) => "install ffmpeg".into(),
        (Need::SceneRenderer, Distro::Arch) => {
            "install linux-wallpaperengine-git from the AUR".into()
        }
        (Need::SceneRenderer, _) => format!("build linux-wallpaperengine from {SCENE_RENDERER}"),
        (Need::Steamcmd, Distro::Arch) => "install steamcmd from the AUR".into(),
        (Need::Steamcmd, Distro::Debian) => format!(
            "{} with i386 packages enabled",
            package("sudo apt install", "steamcmd")
        ),
        (Need::Steamcmd, _) => format!("get steamcmd from {STEAMCMD}"),
    }
}

/// The family, from `ID` and `ID_LIKE`: Ubuntu and Mint are Debian, Manjaro and EndeavourOS
/// are Arch, and so on.
fn distro(os_release: &str) -> Distro {
    let ids: Vec<String> = os_release
        .lines()
        .filter_map(|line| {
            line.strip_prefix("ID=")
                .or_else(|| line.strip_prefix("ID_LIKE="))
        })
        .flat_map(|value| value.trim_matches('"').split_whitespace())
        .map(str::to_lowercase)
        .collect();
    let is = |names: &[&str]| ids.iter().any(|id| names.contains(&id.as_str()));
    if is(&["arch"]) {
        Distro::Arch
    } else if is(&["debian", "ubuntu"]) {
        Distro::Debian
    } else if is(&["fedora", "rhel"]) {
        Distro::Fedora
    } else if is(&["suse", "opensuse"]) || ids.iter().any(|id| id.starts_with("opensuse")) {
        Distro::Suse
    } else {
        Distro::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_come_from_id_and_id_like() {
        assert_eq!(distro("ID=arch\n"), Distro::Arch);
        assert_eq!(distro("ID=endeavouros\nID_LIKE=arch\n"), Distro::Arch);
        assert_eq!(distro("ID=ubuntu\nID_LIKE=debian\n"), Distro::Debian);
        assert_eq!(
            distro("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"),
            Distro::Debian
        );
        assert_eq!(distro("ID=fedora\n"), Distro::Fedora);
        assert_eq!(
            distro("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n"),
            Distro::Suse
        );
        assert_eq!(distro("ID=nixos\n"), Distro::Other);
        assert_eq!(distro(""), Distro::Other);
    }

    #[test]
    fn hints_name_the_package_and_the_command() {
        assert_eq!(
            hint_for(Need::Mpv, Distro::Debian),
            "install libmpv2 (`sudo apt install libmpv2`)"
        );
        assert!(hint_for(Need::SceneRenderer, Distro::Arch).contains("AUR"));
        assert!(hint_for(Need::SceneRenderer, Distro::Debian).contains(SCENE_RENDERER));
        assert!(hint_for(Need::Steamcmd, Distro::Fedora).contains(STEAMCMD));
    }
}
