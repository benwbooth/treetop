import QtQuick


// Minimize, maximize/restore and close, drawn the way the desktop draws them:
// KDE Plasma's Breeze on Linux (round buttons, red close), Windows' wide
// square buttons on Windows. macOS keeps its own traffic lights (the window
// asks for them in the title bar) and doesn't use this.
Row {
    id: wc
    required property var window
    // Colours from the window's palette (treetop has no Theme singleton).
    required property color textColor
    required property color chromeColor
    required property color hoverColor
    required property color dividerColor
    readonly property bool maximized: window.visibility === Window.Maximized
    readonly property string look: Qt.platform.os === "windows" ? "windows" : "breeze"
    spacing: look === "windows" ? 0 : 6
    rightPadding: look === "windows" ? 0 : 8
    leftPadding: look === "windows" ? 0 : 6

    component WinButton: Item {
        id: b
        property string glyph
        property bool danger: false
        signal hit()
        width: wc.look === "windows" ? 42 : 20
        height: wc.look === "windows" ? wc.height : 20
        anchors.verticalCenter: parent.verticalCenter
        readonly property bool hot: area.containsMouse
        // Windows: a full-height block that lights up (red for close).
        Rectangle {
            visible: wc.look === "windows"
            anchors.fill: parent
            color: area.pressed ? (b.danger ? "#b71c1c" : wc.dividerColor) : (b.hot ? (b.danger ? "#e81123" : wc.hoverColor) : "transparent")
        }
        // Breeze: a circle that fills on hover (red for close).
        Rectangle {
            visible: wc.look === "breeze"
            anchors.fill: parent
            radius: width / 2
            color: b.hot ? (b.danger ? "#da4453" : wc.textColor) : "transparent"
            opacity: area.pressed ? 0.8 : 1
        }
        Canvas {
            id: cv
            anchors.centerIn: parent
            width: 14; height: 14
            readonly property color ink: wc.look === "breeze"
                ? (b.hot ? (b.danger ? "white" : wc.chromeColor) : wc.textColor)
                : (b.hot && b.danger ? "white" : wc.textColor)
            onInkChanged: requestPaint()
            Connections { target: b; function onGlyphChanged() { cv.requestPaint() } }
            onPaint: {
                const c = getContext("2d")
                c.reset()
                c.scale(width / 16, height / 16)
                c.strokeStyle = ink
                c.lineWidth = wc.look === "windows" ? 1 : 1.5
                c.lineCap = "round"
                c.lineJoin = "round"
                c.beginPath()
                switch (b.glyph) {
                case "chevronDown": c.moveTo(4, 6.5); c.lineTo(8, 10.5); c.lineTo(12, 6.5); break
                case "chevronUp": c.moveTo(4, 10); c.lineTo(8, 6); c.lineTo(12, 10); break
                case "diamond": c.moveTo(8, 3.5); c.lineTo(12.5, 8); c.lineTo(8, 12.5); c.lineTo(3.5, 8); c.closePath(); break
                case "close": c.moveTo(4.5, 4.5); c.lineTo(11.5, 11.5); c.moveTo(11.5, 4.5); c.lineTo(4.5, 11.5); break
                case "minimize": c.moveTo(3, 8.5); c.lineTo(13, 8.5); break
                case "maximize": c.rect(3.5, 3.5, 9, 9); break
                case "restore": c.rect(3.5, 5.5, 7, 7); c.moveTo(5.5, 5.5); c.lineTo(5.5, 3.5); c.lineTo(12.5, 3.5); c.lineTo(12.5, 10.5); c.lineTo(10.5, 10.5); break
                }
                c.stroke()
            }
        }
        MouseArea { id: area; anchors.fill: parent; hoverEnabled: true; onClicked: b.hit() }
    }

    // Breeze: ⌄ minimize, ⌃ maximize, ◇ restore, ✕ close. Windows: — □ ❐ ✕.
    WinButton {
        glyph: wc.look === "breeze" ? "chevronDown" : "minimize"
        onHit: wc.window.showMinimized()
    }
    WinButton {
        glyph: wc.look === "breeze" ? (wc.maximized ? "diamond" : "chevronUp") : (wc.maximized ? "restore" : "maximize")
        onHit: wc.maximized ? wc.window.showNormal() : wc.window.showMaximized()
    }
    WinButton { glyph: "close"; danger: true; onHit: wc.window.close() }
}
