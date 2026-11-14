import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Fmt.js" as Fmt

// amdgpu_top-style detail for one GPU: engine activity, memory, clocks,
// temperatures, fan, power and the PCIe link, with a history graph.
Rectangle {
    id: panel
    TipGate { id: tipGate }
    property var gpu: ({})
    property int number: 1
    property var history: []
    property var vramHistory: []
    property font font
    property color textColor
    property color dimColor
    property color trackColor
    property color accent: "#c06fd8"
    property color memColor: "#4fb3d9"
    property color warnColor: "#e05a4f"
    signal hideRequested()

    radius: 6
    color: Qt.rgba(accent.r, accent.g, accent.b, 0.06)
    border.color: Qt.rgba(accent.r, accent.g, accent.b, 0.3)
    implicitHeight: col.implicitHeight + 16

    function v(x, unit, digits) {
        return x === null || x === undefined ? "—" : Number(x).toFixed(digits || 0) + unit
    }
    readonly property var engineNames: ["GFX", "Compute", "DMA", "Decode", "Encode"]
    readonly property var engineTips: [
        "Graphics pipeline: 3D rendering, compositing, games.",
        "Compute queues: ROCm/HIP, OpenCL, Vulkan compute, machine learning.",
        "Copy (SDMA) engines moving data between RAM and VRAM.",
        "Video decode (VCN): video playback, hardware-decoded streams.",
        "Video encode (VCN): screen recording, streaming, transcoding.",
    ]

    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.RightButton
        onClicked: (m) => ctx.popup()
    }
    Menu {
        onClosed: tipGate.disarm()
        id: ctx
        MenuItem { text: "Hide GPU " + panel.number; onTriggered: panel.hideRequested() }
    }

    ColumnLayout {
        id: col
        anchors.fill: parent
        anchors.margins: 8
        spacing: 3

        RowLayout {
            Layout.fillWidth: true
            Text {
                text: "▾"
                color: panelHide.hovered ? panel.accent : panel.dimColor
                font: panel.font
                HoverHandler { id: panelHide; cursorShape: Qt.PointingHandCursor }
                TapHandler { onTapped: panel.hideRequested() }
            }
            Text { text: "GPU" + panel.number; font.family: panel.font.family; font.pointSize: panel.font.pointSize; font.bold: true; color: panel.accent }
            Text { text: panel.gpu.name || ""; font: panel.font; color: panel.textColor; Layout.fillWidth: true; elide: Text.ElideRight }
            Text { text: (panel.gpu.link || "") + (panel.gpu.perfLevel ? "  ·  dpm " + panel.gpu.perfLevel : "") + "  ·  " + (panel.gpu.pdev || ""); font: panel.font; color: panel.dimColor }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 14
            // Left: activity and memory bars
            ColumnLayout {
                Layout.fillWidth: true
                Layout.preferredWidth: 3
                spacing: 1
                Meter {
                    Layout.fillWidth: true; font: panel.font; textColor: panel.textColor; trackColor: panel.trackColor
                    label: "Busy"; labelWidth: 8 * panel.font.pointSize
                    segments: [{ value: panel.gpu.busy || 0, color: panel.accent }]
                    text: panel.v(panel.gpu.busy, "%")
                }
                Meter {
                    Layout.fillWidth: true; font: panel.font; textColor: panel.textColor; trackColor: panel.trackColor
                    visible: panel.gpu.memBusy !== null && panel.gpu.memBusy !== undefined
                    label: "Mem bus"; labelWidth: 8 * panel.font.pointSize
                    segments: [{ value: panel.gpu.memBusy || 0, color: panel.memColor }]
                    text: panel.v(panel.gpu.memBusy, "%")
                }
                Repeater {
                    model: 5
                    Meter {
                        Layout.fillWidth: true; font: panel.font; textColor: panel.textColor; trackColor: panel.trackColor
                        label: panel.engineNames[index]; labelWidth: 8 * panel.font.pointSize
                        readonly property real val: panel.gpu.engines ? panel.gpu.engines[index] : 0
                        segments: [{ value: val, color: Qt.lighter(panel.accent, 1 + index * 0.12) }]
                        text: val.toFixed(0) + "%"
                        ToolTip.text: panel.engineTips[index] + " Summed from the processes using this GPU (your own processes unless treetop runs as root)."
                        ToolTip.visible: eh.hovered && tipGate.armed
                        ToolTip.delay: 600
                        HoverHandler { id: eh }
                    }
                }
                // Busy the hardware reports that no process explains.
                Text {
                    Layout.fillWidth: true
                    visible: (panel.gpu.unattributed || 0) >= 20
                    wrapMode: Text.Wrap
                    font: panel.font
                    color: panel.warnColor
                    text: "⚠ " + (panel.gpu.unattributed || 0).toFixed(0) + "% of the busy time isn't from any process treetop can see: "
                        + "ROCm compute work (processes marked ROCm in the engines column), "
                        + "other users' processes without root access, or a stuck GPU job."
                }
                Meter {
                    Layout.fillWidth: true; font: panel.font; textColor: panel.textColor; trackColor: panel.trackColor
                    label: "VRAM"; labelWidth: 8 * panel.font.pointSize
                    maximum: panel.gpu.vramTotal || 1
                    segments: [{ value: panel.gpu.vramUsed || 0, color: "#d070a8" }]
                    text: Fmt.bytes(panel.gpu.vramUsed) + "/" + Fmt.bytes(panel.gpu.vramTotal)
                }
                Meter {
                    Layout.fillWidth: true; font: panel.font; textColor: panel.textColor; trackColor: panel.trackColor
                    visible: (panel.gpu.gttTotal || 0) > 0
                    label: "GTT"; labelWidth: 8 * panel.font.pointSize
                    maximum: panel.gpu.gttTotal || 1
                    segments: [{ value: panel.gpu.gttUsed || 0, color: Qt.darker(panel.memColor, 1.3) }]
                    text: Fmt.bytes(panel.gpu.gttUsed) + "/" + Fmt.bytes(panel.gpu.gttTotal)
                    ToolTip.text: "GTT: system RAM the GPU has mapped (used for buffers that don't fit or don't need to be in VRAM)."
                    ToolTip.visible: gh.hovered && tipGate.armed
                    ToolTip.delay: 600
                    HoverHandler { id: gh }
                }
            }

            // Middle: sensors
            GridLayout {
                Layout.alignment: Qt.AlignTop
                columns: 2
                columnSpacing: 10
                rowSpacing: 1
                Repeater {
                    model: [
                        ["Shader clock", panel.v(panel.gpu.sclk, " MHz")],
                        ["Memory clock", panel.v(panel.gpu.mclk, " MHz")],
                        ["Edge temp", panel.v(panel.gpu.temp, " °C")],
                        ["Junction", panel.v(panel.gpu.tempJunction, " °C")],
                        ["Memory temp", panel.v(panel.gpu.tempMem, " °C")],
                        ["Power", panel.v(panel.gpu.power, " W") + (panel.gpu.powerCap ? " / " + panel.v(panel.gpu.powerCap, " W") : "")],
                        ["Voltage", panel.v(panel.gpu.voltage, " mV")],
                        ["Fan", panel.gpu.fanRpm !== null && panel.gpu.fanRpm !== undefined
                            ? panel.v(panel.gpu.fanRpm, " rpm") + (panel.gpu.fanPct !== null && panel.gpu.fanPct !== undefined ? " (" + panel.v(panel.gpu.fanPct, "%") + ")" : "")
                            : "—"],
                    ]
                    delegate: Repeater {
                        model: 2
                        Text {
                            required property int index
                            readonly property var pair: modelData
                            text: pair[index]
                            font: panel.font
                            color: index === 0 ? panel.dimColor : panel.textColor
                            Layout.alignment: index === 0 ? Qt.AlignLeft : Qt.AlignRight
                        }
                    }
                }
            }

            // Right: history
            Sparkline {
                Layout.fillWidth: true
                Layout.preferredWidth: 2
                Layout.fillHeight: true
                Layout.minimumHeight: 80
                maximum: 100
                series: [{ values: panel.history, color: panel.accent }, { values: panel.vramHistory, color: panel.memColor }]
            }
        }
    }
}
