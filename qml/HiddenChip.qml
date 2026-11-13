import QtQuick
import QtQuick.Layouts

// What a hidden item leaves behind: a faint, clickable "… hidden · show".
Rectangle {
    id: chip
    property string label
    property bool inline: false
    signal restore()

    implicitWidth: (inline ? row.implicitWidth : col.implicitWidth) + 16
    implicitHeight: inline ? row.implicitHeight + 4 : col.implicitHeight + 12
    radius: inline ? 3 : 6
    color: hover.hovered ? Qt.rgba(win.cMagenta.r, win.cMagenta.g, win.cMagenta.b, 0.15) : "transparent"
    border.color: Qt.rgba(win.dimColor.r, win.dimColor.g, win.dimColor.b, inline ? 0 : 0.5)
    border.width: 1

    HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
    TapHandler { onTapped: chip.restore() }

    Column {
        id: col
        visible: !chip.inline
        anchors.centerIn: parent
        spacing: 4
        Text { anchors.horizontalCenter: parent.horizontalCenter; text: chip.label; horizontalAlignment: Text.AlignHCenter; font: win.mono; color: win.dimColor }
        Text { anchors.horizontalCenter: parent.horizontalCenter; text: "+ show"; font.family: win.mono.family; font.pointSize: win.mono.pointSize; font.bold: true; color: win.cMagenta }
    }
    RowLayout {
        id: row
        visible: chip.inline
        anchors.fill: parent
        anchors.leftMargin: 8
        Text { text: "▸ " + chip.label; font: win.mono; color: win.dimColor; Layout.fillWidth: true; elide: Text.ElideRight }
        Text { text: "+ show"; font.family: win.mono.family; font.pointSize: win.mono.pointSize; font.bold: true; color: win.cMagenta }
    }
}
