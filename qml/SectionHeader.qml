import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// A clickable "▾ Title  summary" line that folds a meter section away.
Item {
    id: h
    property string title
    property string summary
    property bool open: true
    // An optional small link at the end of the line (e.g. "+1 hidden").
    property string actionText: ""
    property string actionTip: ""
    signal toggled()
    signal action()
    implicitHeight: row.implicitHeight + 2

    Rectangle {
        anchors.fill: parent
        radius: 3
        color: hover.hovered ? Qt.rgba(win.textColor.r, win.textColor.g, win.textColor.b, 0.07) : "transparent"
    }
    HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
    TapHandler { onTapped: h.toggled() }
    RowLayout {
        id: row
        anchors.fill: parent
        spacing: 6
        Text {
            text: (h.open ? "▾ " : "▸ ") + h.title
            font.family: win.mono.family
            font.pointSize: win.mono.pointSize
            font.bold: true
            color: win.cCyan
        }
        Text {
            Layout.fillWidth: true
            text: h.summary
            font: win.mono
            color: win.dimColor
            elide: Text.ElideRight
        }
        Text {
            visible: h.actionText.length > 0
            text: h.actionText
            font.family: win.mono.family
            font.pointSize: win.mono.pointSize
            font.underline: actHover.hovered
            color: actHover.hovered ? win.cMagenta : win.dimColor
            HoverHandler { id: actHover; cursorShape: Qt.PointingHandCursor }
            TapHandler { onTapped: h.action() }
            ToolTip.visible: actHover.hovered && h.actionTip.length > 0
            ToolTip.text: h.actionTip
            ToolTip.delay: 400
        }
    }
}
