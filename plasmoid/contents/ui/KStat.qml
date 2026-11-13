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
    readonly property bool inside: overlay && (showGraph || bar)

    spacing: Math.round(h * 0.06)

    Text {
        visible: stat.label.length > 0
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
            ? Math.max(valueInside.implicitWidth + 4, Math.round(stat.h * 1.0 * stat.graphWidthFactor))
            : (stat.bar ? Math.round(stat.h * 0.4) : Math.round(stat.h * 1.0 * stat.graphWidthFactor))
        Layout.preferredHeight: Math.round(stat.h * 0.62)
        color: "transparent"
        border.color: Qt.rgba(stat.color.r, stat.color.g, stat.color.b, 0.55)
        border.width: 1
        radius: 1
        Rectangle {
            visible: stat.bar
            anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
            anchors.margins: 1
            height: Math.max(0, Math.min(1, stat.fill)) * (parent.height - 2)
            color: stat.color
        }
        Text {
            id: valueInside
            visible: stat.inside
            z: 2
            anchors.centerIn: parent
            text: stat.value
            color: Kirigami.Theme.textColor
            font.pixelSize: Math.round(stat.h * 0.36)
            font.bold: true
            font.family: "monospace"
            style: Text.Outline
            styleColor: Qt.rgba(Kirigami.Theme.backgroundColor.r, Kirigami.Theme.backgroundColor.g, Kirigami.Theme.backgroundColor.b, 0.85)
            renderType: Text.NativeRendering
        }
        Canvas {
            id: graph
            visible: !stat.bar
            anchors.fill: parent
            anchors.margins: 1
            renderStrategy: Canvas.Threaded
            onPaint: {
                const ctx = getContext("2d")
                ctx.reset()
                const v = stat.history
                if (!v || v.length < 2) return
                const w = width, hh = height, n = v.length
                ctx.beginPath()
                ctx.moveTo(0, hh)
                for (let i = 0; i < n; i++) ctx.lineTo(i / (n - 1) * w, hh - Math.min(1, Math.max(0, v[i])) * hh)
                ctx.lineTo(w, hh)
                ctx.closePath()
                ctx.fillStyle = Qt.rgba(stat.color.r, stat.color.g, stat.color.b, 0.45)
                ctx.fill()
                ctx.beginPath()
                for (let i = 0; i < n; i++) {
                    const y = hh - Math.min(1, Math.max(0, v[i])) * hh
                    if (i === 0) ctx.moveTo(0, y); else ctx.lineTo(i / (n - 1) * w, y)
                }
                ctx.strokeStyle = stat.color
                ctx.lineWidth = 1
                ctx.stroke()
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
}
