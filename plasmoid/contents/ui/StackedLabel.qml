import QtQuick
import QtQuick.Window

// A label written top to bottom with each letter upright. Each letter is
// its own item on a row grid that lands on whole *device* pixels, so with
// fractional scaling (e.g. 130%) native text rendering can't round the rows
// unevenly, and no letter pokes out of its slot.
Item {
    id: sl
    property string text
    property real h: 22
    property color color
    // Rows sized for four letters (VRAM, DISK) so every label matches.
    readonly property real dpr: Screen.devicePixelRatio > 0 ? Screen.devicePixelRatio : 1
    readonly property real row: Math.max(1, Math.floor(h * 0.92 / 4 * dpr)) / dpr
    implicitWidth: Math.ceil(fm.advanceWidth("W") + 1)
    implicitHeight: row * text.length

    FontMetrics { id: fm; font: probe.font }
    Text { id: probe; visible: false; font.pixelSize: Math.max(5, Math.floor(sl.row * 0.8 * sl.dpr) / sl.dpr); font.bold: true }

    Repeater {
        model: sl.text.length
        Text {
            x: 0
            y: Math.round(index * sl.row * sl.dpr) / sl.dpr
            width: sl.implicitWidth
            height: sl.row
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            text: sl.text.charAt(index)
            color: sl.color
            font: probe.font
            renderType: Text.NativeRendering
        }
    }
}
