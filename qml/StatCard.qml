import QtQuick
import QtQuick.Layouts

// kstats-style tile: title, big number, a detail line and a sparkline.
Rectangle {
    id: card
    property string title
    property string value
    property string detail
    property alias series: spark.series
    property alias maximum: spark.maximum
    property alias mirrored: spark.mirrored
    property color accent: "#3daee9"
    property color textColor
    property color dimColor
    property font font
    signal clicked()
    // A ▾ before the title that hides the card (e.g. a GPU).
    property bool collapsible: false
    signal collapseRequested()

    radius: 6
    color: Qt.rgba(accent.r, accent.g, accent.b, hover.hovered ? 0.14 : 0.08)
    border.color: Qt.rgba(accent.r, accent.g, accent.b, 0.35)
    implicitHeight: 92
    implicitWidth: 170

    HoverHandler { id: hover }
    TapHandler { onTapped: card.clicked() }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 8
        spacing: 1
        RowLayout {
            Layout.fillWidth: true
            Text {
                visible: card.collapsible
                text: "▾"
                color: collapseHover.hovered ? card.accent : card.dimColor
                font.pointSize: card.font.pointSize
                font.family: card.font.family
                HoverHandler { id: collapseHover; cursorShape: Qt.PointingHandCursor }
                TapHandler { onTapped: card.collapseRequested() }
            }
            Text { text: card.title; color: card.accent; font.bold: true; font.pointSize: card.font.pointSize; font.family: card.font.family; Layout.fillWidth: true; elide: Text.ElideRight }
            Text { text: card.value; color: card.textColor; font.bold: true; font.pointSize: card.font.pointSize * 1.25; font.family: card.font.family }
        }
        Text { text: card.detail; color: card.dimColor; font: card.font; Layout.fillWidth: true; elide: Text.ElideRight }
        Sparkline { id: spark; Layout.fillWidth: true; Layout.fillHeight: true }
    }
}
