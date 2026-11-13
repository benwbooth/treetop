import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// One panel item: a label, a value and a small history graph behind it.
Item {
    id: stat
    property string label
    property string value
    property var history: []       // 0..1 values (or two series for in/out)
    property var history2: []
    property color color: Kirigami.Theme.positiveTextColor
    property color color2: Kirigami.Theme.negativeTextColor
    property bool showGraph: true

    implicitWidth: Math.max(labelText.implicitWidth, valueText.implicitWidth) + Kirigami.Units.smallSpacing * 2
    implicitHeight: labelText.implicitHeight + valueText.implicitHeight

    Canvas {
        id: graph
        anchors.fill: parent
        visible: stat.showGraph
        opacity: 0.55
        onPaint: {
            const ctx = getContext("2d")
            ctx.reset()
            const draw = (vals, col) => {
                if (!vals || vals.length < 2) return
                const n = vals.length, w = width, h = height
                ctx.beginPath()
                ctx.moveTo(0, h)
                for (let i = 0; i < n; i++) ctx.lineTo(i / (n - 1) * w, h - Math.min(1, vals[i]) * h)
                ctx.lineTo(w, h)
                ctx.closePath()
                ctx.fillStyle = Qt.rgba(col.r, col.g, col.b, 0.45)
                ctx.fill()
            }
            draw(stat.history, stat.color)
            draw(stat.history2, stat.color2)
        }
    }
    onHistoryChanged: graph.requestPaint()

    Column {
        anchors.centerIn: parent
        spacing: -2
        Text {
            id: labelText
            anchors.horizontalCenter: parent.horizontalCenter
            text: stat.label
            color: Kirigami.Theme.disabledTextColor
            font.pixelSize: Math.max(7, stat.height * 0.36)
            font.bold: true
            renderType: Text.NativeRendering
        }
        Text {
            id: valueText
            anchors.horizontalCenter: parent.horizontalCenter
            text: stat.value
            color: Kirigami.Theme.textColor
            font.family: "monospace"
            font.pixelSize: Math.max(8, stat.height * 0.42)
            renderType: Text.NativeRendering
        }
    }
}
