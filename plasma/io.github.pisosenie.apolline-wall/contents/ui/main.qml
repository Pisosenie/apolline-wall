// One monitor of apolline-wall inside Plasma's desktop, under its icons and widgets: the
// daemon draws the monitor into a PipeWire stream, and this plays it. It asks for the stream
// with `apolline-wall feed`, often until frames come and now and then after, to find the new
// node when the daemon restarts, and tells it whenever windows come to hide the desktop, so
// the monitor pauses unseen.

import QtQuick
import QtQuick.Window
import org.kde.pipewire as PipeWire
import org.kde.plasma.plasma5support as P5Support
import org.kde.plasma.plasmoid

WallpaperItem {
    id: root

    // The monitor's connector, the name apolline-wall knows it by. Anything else never
    // reaches a command line.
    readonly property string output: /^[A-Za-z0-9._-]+$/.test(Screen.name) ? Screen.name : ""
    property int node: 0
    property string stream: ""
    readonly property bool covered: windows.item ? windows.item.covered : false

    function ask() {
        if (output !== "") {
            daemon.connectSource("apolline-wall feed " + output + " --covered " + covered)
        }
    }

    onCoveredChanged: ask()

    Rectangle {
        anchors.fill: parent
        color: "black"
    }

    // Made only once there is a node: KPipeWire doesn't take node 0 well.
    Loader {
        id: frames
        anchors.fill: parent
        active: root.node > 0
        sourceComponent: PipeWire.PipeWireSourceItem {
            nodeId: root.node
        }
    }

    P5Support.DataSource {
        id: daemon
        engine: "executable"
        onNewData: (source, data) => {
            disconnectSource(source)
            const [node, stream] = data["exit code"] === 0 ? data.stdout.trim().split(" ") : []
            // A new stream may come on the id of the old one: start the player afresh.
            if (parseInt(node, 10) > 0 && stream && stream !== root.stream) {
                root.node = 0
                root.stream = stream
                root.node = parseInt(node, 10)
            }
        }
    }

    Loader {
        id: windows
        source: "Covered.qml"
        onLoaded: item.regionGeometry = Qt.binding(() => Window.window
            ? Qt.rect(Window.window.x, Window.window.y,
                      Window.window.width, Window.window.height)
            : Qt.rect(0, 0, 0, 0))
    }

    Timer {
        interval: frames.item && frames.item.ready ? 15000 : 2000
        running: root.output !== ""
        repeat: true
        triggeredOnStart: true
        onTriggered: root.ask()
    }
}
