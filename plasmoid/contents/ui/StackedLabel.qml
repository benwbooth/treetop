import QtQuick
import QtQuick.Window

// A label written top to bottom with each letter upright. Each letter is
// its own item on a row grid that lands on whole *device* pixels, so with
// fractional scaling (e.g. 130%) native text rendering can't round the rows
// unevenly, and no letter pokes out of its slot.
//
// The font is sized by its capital height, not its em: the labels are all
// capitals, and an em-sized font left them half the row tall (4-5 px
// letters, which hinting mangles). Now the capitals fill about 70% of
// the row (room between letters), each on a whole-pixel baseline.
Item {
    id: sl
    property string text
    property real h: 22
    property color color
    // Rows sized for four letters (VRAM, DISK) so every label matches.
    readonly property real dpr: Screen.devicePixelRatio > 0 ? Screen.devicePixelRatio : 1
    readonly property int rowDev: Math.max(1, Math.floor(h * 0.92 / 4 * dpr))
    readonly property real row: rowDev / dpr
    // Capital height per pixel of font size, measured on the real font.
    readonly property real capRatio: Math.max(0.5, Math.min(0.9, ref.tightBoundingRect("M").height / 100))
    readonly property int gapDev: Math.max(2, Math.round(rowDev * 0.3))
    readonly property real capH: Math.max(4, rowDev - gapDev) / dpr
    implicitWidth: Math.ceil(fm.advanceWidth("W") + 1)
    implicitHeight: row * text.length

    FontMetrics { id: ref; font.bold: true; font.pixelSize: 100 }
    FontMetrics { id: fm; font: probe.font }
    Text { id: probe; visible: false; font.pixelSize: Math.max(5, Math.round(sl.capH / sl.capRatio * sl.dpr) / sl.dpr); font.bold: true }

    Repeater {
        model: sl.text.length
        Text {
            // The baseline: the capital centred in its row, on a whole
            // device pixel.
            readonly property real base: Math.round((index * sl.row + (sl.row + sl.capH) / 2) * sl.dpr) / sl.dpr
            x: 0
            y: base - fm.ascent
            width: sl.implicitWidth
            horizontalAlignment: Text.AlignHCenter
            text: sl.text.charAt(index)
            color: sl.color
            font: probe.font
            renderType: Text.NativeRendering
        }
    }
}
