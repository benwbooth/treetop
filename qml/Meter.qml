import QtQuick

// One htop meter: "Label [|||||||      42.0%]", drawn as coloured segments.
Item {
    id: meter
    property string label
    property var segments: []   // [{ value: 0..max, color }]
    property real maximum: 100
    property string text
    property font font
    property color textColor
    property color trackColor
    property real labelWidth: fm.advanceWidth("CPU00 ")

    implicitHeight: fm.height + 2
    FontMetrics { id: fm; font: meter.font }

    Text {
        id: lbl
        width: meter.labelWidth
        anchors.verticalCenter: parent.verticalCenter
        text: meter.label
        font: meter.font
        color: "#4fb3d9"
        elide: Text.ElideMiddle
    }
    Rectangle {
        id: track
        anchors.left: lbl.right
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        height: fm.height - 2
        radius: 2
        color: meter.trackColor
        clip: true
        Row {
            anchors.fill: parent
            Repeater {
                // A count, not the array: the segments are created once and
                // only resized as values change.
                model: meter.segments.length
                Rectangle {
                    readonly property var seg: meter.segments[index] || ({ value: 0, color: "transparent" })
                    height: track.height
                    width: Math.max(0, Math.min(1, (seg.value || 0) / meter.maximum)) * track.width
                    color: seg.color
                }
            }
        }
        Text {
            anchors.right: parent.right
            anchors.rightMargin: 4
            anchors.verticalCenter: parent.verticalCenter
            text: meter.text
            font: meter.font
            color: meter.textColor
            style: Text.Outline
            styleColor: Qt.rgba(meter.trackColor.r, meter.trackColor.g, meter.trackColor.b, 0.6)
        }
    }
}
