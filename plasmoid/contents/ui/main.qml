import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ksysguard.sensors as Sensors
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.plasmoid
import org.kde.plasma.plasma5support as P5Support

// treetop's panel widget: kstats-style live mini graphs from KDE's sensor
// daemon. Clicking toggles the treetop window (treetop --toggle).
PlasmoidItem {
    id: root
    readonly property int rate: Math.max(250, Plasmoid.configuration.updateInterval)
    readonly property int historyLength: 30
    preferredRepresentation: compactRepresentation
    Plasmoid.title: "treetop"
    Plasmoid.icon: "utilities-system-monitor"

    // ---- sensors ---------------------------------------------------------
    Sensors.Sensor { id: cpu; sensorId: "cpu/all/usage"; updateRateLimit: root.rate }
    Sensors.Sensor { id: cpuTemp; sensorId: "cpu/all/averageTemperature"; updateRateLimit: root.rate }
    Sensors.Sensor { id: mem; sensorId: "memory/physical/usedPercent"; updateRateLimit: root.rate }
    Sensors.Sensor { id: memUsed; sensorId: "memory/physical/used"; updateRateLimit: root.rate }
    Sensors.Sensor { id: memTotal; sensorId: "memory/physical/total" }
    Sensors.Sensor { id: diskR; sensorId: "disk/all/read"; updateRateLimit: root.rate }
    Sensors.Sensor { id: diskW; sensorId: "disk/all/write"; updateRateLimit: root.rate }
    Sensors.Sensor { id: netDown; sensorId: "network/all/download"; updateRateLimit: root.rate }
    Sensors.Sensor { id: netUp; sensorId: "network/all/upload"; updateRateLimit: root.rate }

    // GPUs: ksystemstats numbers them gpu0, gpu1, ...
    Sensors.SensorTreeModel { id: tree }
    property int gpuCount: 0
    Timer {
        // Count GPUs once the sensor tree has loaded.
        interval: 1500; running: true; repeat: false
        onTriggered: {
            let n = 0
            while (n < 8) {
                const probe = Qt.createQmlObject('import org.kde.ksysguard.sensors as S; S.Sensor { sensorId: "gpu/gpu' + n + '/usage" }', root)
                const ok = probe.name && probe.name.length > 0
                probe.destroy()
                if (!ok) break
                n++
            }
            root.gpuCount = n
        }
    }
    readonly property var gpuIndexes: {
        const out = []
        for (let i = 0; i < gpuCount; i++)
            if (Plasmoid.configuration.hiddenGpus.indexOf(String(i)) < 0) out.push(i)
        return out
    }
    Instantiator {
        id: gpuSensors
        model: root.gpuCount
        delegate: QtObject {
            required property int index
            property var usage: Sensors.Sensor { sensorId: "gpu/gpu" + index + "/usage"; updateRateLimit: root.rate }
            property var vram: Sensors.Sensor { sensorId: "gpu/gpu" + index + "/usedVram"; updateRateLimit: root.rate }
            property var vramTotal: Sensors.Sensor { sensorId: "gpu/gpu" + index + "/totalVram" }
            property var temp: Sensors.Sensor { sensorId: "gpu/gpu" + index + "/temperature"; updateRateLimit: root.rate }
            property var name: Sensors.Sensor { sensorId: "gpu/gpu" + index + "/name" }
            property var hist: []
        }
    }

    // ---- history ---------------------------------------------------------
    property var hCpu: []
    property var hMem: []
    property var hDiskR: []
    property var hDiskW: []
    property var hDown: []
    property var hUp: []
    property real peakDisk: 1
    property real peakNet: 1
    property int tick: 0
    function push(a, v) { const b = a.slice(a.length >= historyLength ? 1 : 0); b.push(Math.max(0, v || 0)); return b }
    function norm(a, peak) { return a.map(v => v / peak) }
    Timer {
        interval: root.rate; running: true; repeat: true
        onTriggered: {
            root.hCpu = root.push(root.hCpu, cpu.value / 100)
            root.hMem = root.push(root.hMem, mem.value / 100)
            root.hDiskR = root.push(root.hDiskR, diskR.value)
            root.hDiskW = root.push(root.hDiskW, diskW.value)
            root.hDown = root.push(root.hDown, netDown.value)
            root.hUp = root.push(root.hUp, netUp.value)
            root.peakDisk = Math.max(1048576, ...root.hDiskR, ...root.hDiskW)
            root.peakNet = Math.max(131072, ...root.hDown, ...root.hUp)
            for (let i = 0; i < gpuSensors.count; i++) {
                const g = gpuSensors.objectAt(i)
                if (g) g.hist = root.push(g.hist, g.usage.value / 100)
            }
            root.tick++
        }
    }

    function rate(bytes) {
        bytes = Number(bytes) || 0
        const u = ["B", "K", "M", "G"]
        let i = 0
        while (bytes >= 1024 && i < 3) { bytes /= 1024; i++ }
        return (bytes < 10 && i > 0 ? bytes.toFixed(1) : bytes.toFixed(0)) + u[i]
    }
    function pct(s) { return (Number(s.value) || 0).toFixed(0) + "%" }

    // ---- open treetop ----------------------------------------------------
    P5Support.DataSource {
        id: exec
        engine: "executable"
        connectedSources: []
        onNewData: (source) => disconnectSource(source)
    }
    function toggleTreetop() {
        exec.connectSource("treetop --toggle || devloop start treetop")
    }

    toolTipMainText: "treetop"
    toolTipSubText: {
        tick
        let t = "CPU " + pct(cpu) + (cpuTemp.value ? " · " + Number(cpuTemp.value).toFixed(0) + "°C" : "")
        t += "\nMemory " + pct(mem) + " · " + memUsed.formattedValue + " of " + memTotal.formattedValue
        for (const i of gpuIndexes) {
            const g = gpuSensors.objectAt(i)
            if (g) t += "\nGPU" + (i + 1) + " " + pct(g.usage) + " · VRAM " + g.vram.formattedValue + (g.temp.value ? " · " + Number(g.temp.value).toFixed(0) + "°C" : "") + "  (" + g.name.value + ")"
        }
        t += "\nDisk read " + rate(diskR.value) + "/s · write " + rate(diskW.value) + "/s"
        t += "\nNetwork ↓ " + rate(netDown.value) + "/s · ↑ " + rate(netUp.value) + "/s"
        return t + "\n\nClick to open treetop"
    }

    Plasmoid.contextualActions: [
        PlasmaCore.Action { text: "Open treetop"; icon.name: "utilities-system-monitor"; onTriggered: root.toggleTreetop() },
        PlasmaCore.Action { isSeparator: true },
        PlasmaCore.Action { text: "Show CPU"; checkable: true; checked: Plasmoid.configuration.showCpu; onTriggered: Plasmoid.configuration.showCpu = checked },
        PlasmaCore.Action { text: "Show memory"; checkable: true; checked: Plasmoid.configuration.showMemory; onTriggered: Plasmoid.configuration.showMemory = checked },
        PlasmaCore.Action { text: "Show GPU 1"; checkable: true; visible: root.gpuCount > 0; checked: Plasmoid.configuration.hiddenGpus.indexOf("0") < 0; onTriggered: root.setGpu(0, checked) },
        PlasmaCore.Action { text: "Show GPU 2"; checkable: true; visible: root.gpuCount > 1; checked: Plasmoid.configuration.hiddenGpus.indexOf("1") < 0; onTriggered: root.setGpu(1, checked) },
        PlasmaCore.Action { text: "Show GPU 3"; checkable: true; visible: root.gpuCount > 2; checked: Plasmoid.configuration.hiddenGpus.indexOf("2") < 0; onTriggered: root.setGpu(2, checked) },
        PlasmaCore.Action { text: "Show disk"; checkable: true; checked: Plasmoid.configuration.showDisk; onTriggered: Plasmoid.configuration.showDisk = checked },
        PlasmaCore.Action { text: "Show network"; checkable: true; checked: Plasmoid.configuration.showNetwork; onTriggered: Plasmoid.configuration.showNetwork = checked },
        PlasmaCore.Action { text: "Graphs behind values"; checkable: true; checked: Plasmoid.configuration.showGraphs; onTriggered: Plasmoid.configuration.showGraphs = checked }
    ]
    function setGpu(i, show) {
        const h = Plasmoid.configuration.hiddenGpus.filter(x => x !== String(i))
        if (!show) h.push(String(i))
        Plasmoid.configuration.hiddenGpus = h
    }

    // Inside the system tray the widget gets an icon-sized slot.
    readonly property bool inTray: Plasmoid.containment !== null
        && String(Plasmoid.containment.pluginName).indexOf("systemtray") >= 0

    compactRepresentation: MouseArea {
        id: compact
        readonly property bool vertical: Plasmoid.formFactor === PlasmaCore.Types.Vertical
        Layout.minimumWidth: root.inTray ? -1 : (vertical ? -1 : grid.implicitWidth)
        Layout.preferredWidth: root.inTray ? -1 : (vertical ? -1 : grid.implicitWidth)
        Layout.minimumHeight: root.inTray ? -1 : (vertical ? grid.implicitHeight : -1)
        Layout.preferredHeight: root.inTray ? -1 : (vertical ? grid.implicitHeight : -1)
        hoverEnabled: true
        onClicked: root.toggleTreetop()

        // Tray look: four live bars (CPU, memory, GPU, network) in the icon.
        Row {
            visible: root.inTray
            anchors.centerIn: parent
            readonly property real s: Math.min(compact.width, compact.height)
            width: s
            height: s
            spacing: Math.max(1, s * 0.06)
            Repeater {
                model: [
                    { c: "#43c96b", v: () => cpu.value / 100 },
                    { c: "#d6b23e", v: () => mem.value / 100 },
                    { c: "#c06fd8", v: () => { const i = root.gpuIndexes.length ? root.gpuIndexes[root.gpuIndexes.length - 1] : -1; const g = i >= 0 ? gpuSensors.objectAt(i) : null; return g ? g.usage.value / 100 : 0 } },
                    { c: "#4f8fe0", v: () => (Number(netDown.value) + Number(netUp.value)) / root.peakNet },
                ]
                Item {
                    required property var modelData
                    width: (parent.s - parent.spacing * 3) / 4
                    height: parent.s
                    Rectangle {
                        anchors.fill: parent
                        radius: 1
                        color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.15)
                    }
                    Rectangle {
                        anchors.bottom: parent.bottom
                        width: parent.width
                        radius: 1
                        color: modelData.c
                        height: { root.tick; return Math.max(1, Math.min(1, modelData.v() || 0) * parent.height) }
                    }
                }
            }
        }

        GridLayout {
            id: grid
            visible: !root.inTray
            anchors.fill: parent
            flow: compact.vertical ? GridLayout.TopToBottom : GridLayout.LeftToRight
            rows: compact.vertical ? -1 : 1
            columns: compact.vertical ? 1 : -1
            columnSpacing: Kirigami.Units.smallSpacing
            rowSpacing: Kirigami.Units.smallSpacing
            readonly property real itemSize: compact.vertical ? compact.width : compact.height

            MiniStat {
                visible: Plasmoid.configuration.showCpu
                Layout.fillHeight: !compact.vertical
                Layout.preferredWidth: compact.vertical ? -1 : Math.max(implicitWidth, grid.itemSize * 1.6)
                Layout.preferredHeight: compact.vertical ? grid.itemSize * 0.7 : -1
                label: "CPU"; value: root.pct(cpu)
                history: root.hCpu; color: "#43c96b"
                showGraph: Plasmoid.configuration.showGraphs
            }
            MiniStat {
                visible: Plasmoid.configuration.showMemory
                Layout.fillHeight: !compact.vertical
                Layout.preferredWidth: compact.vertical ? -1 : Math.max(implicitWidth, grid.itemSize * 1.6)
                Layout.preferredHeight: compact.vertical ? grid.itemSize * 0.7 : -1
                label: "MEM"; value: root.pct(mem)
                history: root.hMem; color: "#d6b23e"
                showGraph: Plasmoid.configuration.showGraphs
            }
            Repeater {
                model: root.gpuIndexes
                MiniStat {
                    required property int modelData
                    readonly property var g: { root.tick; return gpuSensors.objectAt(modelData) }
                    Layout.fillHeight: !compact.vertical
                    Layout.preferredWidth: compact.vertical ? -1 : Math.max(implicitWidth, grid.itemSize * 1.6)
                    Layout.preferredHeight: compact.vertical ? grid.itemSize * 0.7 : -1
                    label: "GPU" + (modelData + 1); value: g ? root.pct(g.usage) : "–"
                    history: g ? g.hist : []; color: "#c06fd8"
                    showGraph: Plasmoid.configuration.showGraphs
                }
            }
            MiniStat {
                visible: Plasmoid.configuration.showDisk
                Layout.fillHeight: !compact.vertical
                Layout.preferredWidth: compact.vertical ? -1 : Math.max(implicitWidth, grid.itemSize * 1.8)
                Layout.preferredHeight: compact.vertical ? grid.itemSize * 0.7 : -1
                label: "DISK"; value: root.rate(Number(diskR.value) + Number(diskW.value))
                history: root.norm(root.hDiskR, root.peakDisk); history2: root.norm(root.hDiskW, root.peakDisk)
                color: "#4fb3d9"; color2: "#e0894f"
                showGraph: Plasmoid.configuration.showGraphs
            }
            MiniStat {
                visible: Plasmoid.configuration.showNetwork
                Layout.fillHeight: !compact.vertical
                Layout.preferredWidth: compact.vertical ? -1 : Math.max(implicitWidth, grid.itemSize * 1.8)
                Layout.preferredHeight: compact.vertical ? grid.itemSize * 0.7 : -1
                label: "NET"; value: "↓" + root.rate(netDown.value)
                history: root.norm(root.hDown, root.peakNet); history2: root.norm(root.hUp, root.peakNet)
                color: "#4f8fe0"; color2: "#e05a4f"
                showGraph: Plasmoid.configuration.showGraphs
            }
        }
    }
}
