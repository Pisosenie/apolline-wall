// Whether windows hide this desktop, decided the way Plasma's own animated wallpapers decide
// it: a maximized or fullscreen window on the current desktop and activity, unless the desktop
// is being shown. Loaded apart, so a Plasma without this type still shows the wallpaper.

import QtQuick
import org.kde.kwindowsystem
import org.kde.plasma.wallpapers.image as PlasmaWallpaper

PlasmaWallpaper.MaximizedWindowMonitor {
    readonly property bool covered: count > 0 && !KWindowSystem.showingDesktop
}
