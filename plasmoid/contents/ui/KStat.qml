import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// One KStats-style panel item: coloured label, a small framed history graph
// (or a fill bar), and the value.
RowLayout {
    id: stat
    property string label
    property color color: "#43c96b"
    property string value
    property var history: []       // 0..1
    property bool showGraph: true
    property bool bar: false       // a fill bar instead of a history graph
    property real fill: 0          // 0..1, for the bar
    property bool colorLabel: true
    property real graphWidthFactor: 1
    property real h: 22
    // Draw the value on top of the graph (compact) instead of beside it.
    property bool overlay: true
    // Write the label sideways (bottom to top), so a full name takes the
    // width of one small letter.
    property bool verticalLabel: false
    // Or stack the letters top to bottom, each one upright.
    property bool stackedLabel: false
    readonly property bool inside: overlay && (showGraph || bar)

    spacing: Math.round(h * 0.06)

    Text {
        visible: stat.stackedLabel && stat.label.length > 0
        Layout.alignment: Qt.AlignVCenter
        text: stat.label.split("").join("\n")
        horizontalAlignment: Text.AlignHCenter
        // Fixed line height sized for four letters (VRAM, DISK), the same
        // for every label so they look even.
        lineHeightMode: Text.FixedHeight
        lineHeight: Math.floor(stat.h * 0.92 / 4)
        color: stat.colorLabel ? stat.color : Kirigami.Theme.textColor
        font.pixelSize: Math.max(5, Math.floor(stat.h * 0.92 / 4))
        font.bold: true
        renderType: Text.NativeRendering
    }
    Item {
        visible: stat.verticalLabel && stat.label.length > 0
        Layout.alignment: Qt.AlignVCenter
        implicitWidth: vlabel.implicitHeight
        implicitHeight: Math.min(stat.h, vlabel.implicitWidth)
        Text {
            id: vlabel
            anchors.centerIn: parent
            rotation: -90
            text: stat.label
            color: stat.colorLabel ? stat.color : Kirigami.Theme.textColor
            font.pixelSize: Math.max(6, Math.round(stat.h * 0.2))
            font.bold: true
            renderType: Text.NativeRendering
        }
    }
    Text {
        visible: !stat.verticalLabel && !stat.stackedLabel && stat.label.length > 0
        text: stat.label
        color: stat.colorLabel ? stat.color : Kirigami.Theme.textColor
        font.pixelSize: Math.max(7, Math.round(stat.h * 0.22))
        font.bold: true
        renderType: Text.NativeRendering
        Layout.alignment: Qt.AlignVCenter
    }
    Rectangle {
        visible: stat.showGraph || stat.bar
        Layout.alignment: Qt.AlignVCenter
        Layout.preferredWidth: stat.inside
            ? Math.max(valueInside.implicitWidth + 4, Math.round(stat.h * 1.56 * stat.graphWidthFactor))
            : (stat.bar ? Math.round(stat.h * 0.4) : Math.round(stat.h * 1.56 * stat.graphWidthFactor))
        Layout.preferredHeight: Math.round(stat.h * 0.8)
        color: "transparent"
        border.color: Qt.rgba(stat.color.r, stat.color.g, stat.color.b, 0.55)
        border.width: 1
        radius: 1
        Rectangle {
            visible: stat.bar && !stat.inside
            anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
            anchors.margins: 1
            height: Math.max(0, Math.min(1, stat.fill)) * (parent.height - 2)
            color: Qt.rgba(stat.color.r, stat.color.g, stat.color.b, 0.7)
        }
        // Only measures the value's width; the graph canvas paints it.
        Text {
            id: valueInside
            visible: false
            text: stat.value
            font.pixelSize: Math.round(stat.h * 0.3)
            font.bold: true
            font.family: "monospace"
        }
        // Graph (or disk fill) plus the value in the lower-left corner.
        Canvas {
            id: graph
            visible: !stat.bar || stat.inside
            anchors.fill: parent
            anchors.margins: 1
            renderStrategy: Canvas.Threaded
            function shape(ctx, w, hh) {
                ctx.beginPath()
                if (stat.bar) {
                    const fh = Math.max(0, Math.min(1, stat.fill)) * hh
                    ctx.rect(0, hh - fh, w, fh)
                    return fh > 0
                }
                const v = stat.history
                if (!v || v.length < 2) return false
                const n = v.length
                ctx.moveTo(0, hh)
                for (let i = 0; i < n; i++) ctx.lineTo(i / (n - 1) * w, hh - Math.min(1, Math.max(0, v[i])) * hh)
                ctx.lineTo(w, hh)
                ctx.closePath()
                return true
            }
            onPaint: {
                const ctx = getContext("2d")
                ctx.reset()
                const w = width, hh = height
                const has = shape(ctx, w, hh)
                if (has) {
                    ctx.fillStyle = Qt.rgba(stat.color.r, stat.color.g, stat.color.b, stat.bar ? 0.75 : 0.6)
                    ctx.fill()
                }
                const v = stat.history
                if (!stat.bar && v && v.length >= 2) {
                    ctx.beginPath()
                    for (let i = 0; i < v.length; i++) {
                        const y = hh - Math.min(1, Math.max(0, v[i])) * hh
                        if (i === 0) ctx.moveTo(0, y); else ctx.lineTo(i / (v.length - 1) * w, y)
                    }
                    ctx.strokeStyle = Qt.lighter(stat.color, 1.25)
                    ctx.lineWidth = 1.5
                    ctx.stroke()
                }
                if (!stat.inside) return
                // Light digits with a dark halo: readable over the fill and
                // over the empty background alike.
                ctx.font = "bold " + Math.round(stat.h * 0.3) + "px monospace"
                // Lower left: over the oldest history, leaving the current end
                // of the curve (the right edge) in view.
                ctx.textAlign = "left"
                ctx.textBaseline = "bottom"
                const x = 1, y = hh + 1
                const bg = Kirigami.Theme.backgroundColor
                ctx.lineJoin = "round"
                ctx.lineWidth = 3
                ctx.strokeStyle = Qt.rgba(bg.r * 0.6, bg.g * 0.6, bg.b * 0.6, 0.92)
                ctx.strokeText(stat.value, x, y)
                ctx.fillStyle = Qt.lighter(Kirigami.Theme.textColor, 1.1)
                ctx.fillText(stat.value, x, y)
            }
        }
    }
    Text {
        visible: !stat.inside
        text: stat.value
        color: Kirigami.Theme.textColor
        font.pixelSize: Math.round(stat.h * 0.36)
        font.family: "monospace"
        renderType: Text.NativeRendering
        Layout.alignment: Qt.AlignVCenter
    }
    onHistoryChanged: graph.requestPaint()
    onValueChanged: graph.requestPaint()
    onFillChanged: graph.requestPaint()
}
