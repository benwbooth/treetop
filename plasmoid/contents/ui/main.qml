import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ksysguard.sensors as Sensors
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.plasmoid
import org.kde.plasma.plasma5support as P5Support

// treetop's panel widget, KStats-style: CPU, memory, each GPU, disk and
// network with small history graphs, from KDE's sensor daemon. Clicking
// toggles the treetop window. In the system tray it draws four tiny bars.
PlasmoidItem {
    id: root
    readonly property var cfg: Plasmoid.configuration
    readonly property int updateRate: Math.max(250, cfg.updateInterval)
    readonly property int historyLength: 30
    preferredRepresentation: compactRepresentation
    Plasmoid.title: "treetop"
    Plasmoid.icon: "utilities-system-monitor"

    // ---- sensors ---------------------------------------------------------
    Sensors.Sensor { id: cpu; sensorId: "cpu/all/usage"; updateRateLimit: root.updateRate }
    Sensors.Sensor { id: cpuTemp; sensorId: "cpu/all/averageTemperature"; updateRateLimit: root.updateRate }
    Sensors.Sensor { id: mem; sensorId: "memory/physical/usedPercent"; updateRateLimit: root.updateRate }
    Sensors.Sensor { id: memUsed; sensorId: "memory/physical/used"; updateRateLimit: root.updateRate }
    Sensors.Sensor { id: memTotal; sensorId: "memory/physical/total" }
    Sensors.Sensor { id: diskUse; sensorId: root.cfg.diskSensor || "disk/all/usedPercent"; updateRateLimit: 5000 }
    Sensors.Sensor { id: diskR; sensorId: "disk/all/read"; updateRateLimit: root.updateRate }
    Sensors.Sensor { id: diskW; sensorId: "disk/all/write"; updateRateLimit: root.updateRate }
    Sensors.Sensor { id: netDown; sensorId: "network/all/download"; updateRateLimit: root.updateRate }
    Sensors.Sensor { id: netUp; sensorId: "network/all/upload"; updateRateLimit: root.updateRate }

    // GPUs: ksystemstats numbers them gpu0, gpu1, ... Sensors load
    // asynchronously, so a slot counts once its name sensor has a value.
    readonly property int maxGpus: 4
    // Which DRM cards drive a connected display (ksystemstats' gpuN is
    // cardN). Re-read now and then, for monitor hotplugs.
    property var displayCards: []
    P5Support.DataSource {
        id: displayProbe
        engine: "executable"
        connectedSources: []
        onNewData: (source, data) => {
            const cards = []
            for (const line of String(data.stdout || "").split("\n")) {
                const m = line.match(/\/card(\d+)-[^/]+\/status:connected$/)
                if (m && cards.indexOf(Number(m[1])) < 0) cards.push(Number(m[1]))
            }
            root.displayCards = cards
            disconnectSource(source)
        }
    }
    Timer {
        interval: 30000; running: true; repeat: true; triggeredOnStart: true
        onTriggered: displayProbe.connectSource("grep -H . /sys/class/drm/card*-*/status")
    }

    // Sensor slots of the GPUs present, ordered like the treetop app: the
    // one driving the display first, then by VRAM. "GPU1" is the same card
    // everywhere.
    readonly property var gpuPresent: {
        tick
        const out = []
        for (let i = 0; i < maxGpus; i++) {
            const g = gpuSensors.objectAt(i)
            if (g && g.name.value !== undefined && String(g.name.value).length > 0) out.push(i)
        }
        const vram = (i) => Number(gpuSensors.objectAt(i).vramTotal.value) || 0
        const disp = (i) => displayCards.indexOf(i) >= 0 ? 1 : 0
        return out.sort((a, b) => (disp(b) - disp(a)) || (vram(b) - vram(a)))
    }
    // Idle for the last ~30 samples and not driving a display: folded away.
    function gpuIdle(slot) {
        if (!cfg.hideIdleGpus || displayCards.indexOf(slot) >= 0) return false
        const g = gpuSensors.objectAt(slot)
        if (!g || g.hist.length < 20) return false
        const vram = (Number(g.usedVram.value) || 0) / Math.max(1, Number(g.vramTotal.value) || 0)
        return Math.max(...g.hist) < 0.03 && vram < 0.1
    }
    readonly property int gpuCount: gpuPresent.length
    // Display number (1-based) of a sensor slot.
    function gpuNumber(slot) { return gpuPresent.indexOf(slot) + 1 }
    // hiddenGpus holds display positions ("0" = GPU1).
    readonly property var gpuIndexes: cfg.showGpu ? gpuPresent.filter((slot, k) => cfg.hiddenGpus.indexOf(String(k)) < 0 && !gpuIdle(slot)) : []
    Instantiator {
        id: gpuSensors
        model: root.maxGpus
        delegate: QtObject {
            required property int index
            property var usage: Sensors.Sensor { sensorId: "gpu/gpu" + index + "/usage"; updateRateLimit: root.updateRate }
            property var vram: Sensors.Sensor { sensorId: "gpu/gpu" + index + "/usedVram"; updateRateLimit: root.updateRate }
            property var temp: Sensors.Sensor { sensorId: "gpu/gpu" + index + "/temperature"; updateRateLimit: root.updateRate }
            property var name: Sensors.Sensor { sensorId: "gpu/gpu" + index + "/name" }
            property var vramTotal: Sensors.Sensor { sensorId: "gpu/gpu" + index + "/totalVram" }
            property var usedVram: Sensors.Sensor { sensorId: "gpu/gpu" + index + "/usedVram"; updateRateLimit: root.updateRate }
            property var hist: []
            property var vhist: []
        }
    }

    // ---- history ---------------------------------------------------------
    property var hCpu: []
    property var hMem: []
    property var hDisk: []
    property var hNet: []
    property real peakDisk: 1
    property real peakNet: 1
    property int tick: 0
    function push(a, v) { const b = a.slice(a.length >= historyLength ? 1 : 0); b.push(Math.max(0, v || 0)); return b }
    Timer {
        interval: root.updateRate; running: true; repeat: true
        onTriggered: {
            root.hCpu = root.push(root.hCpu, cpu.value / 100)
            root.hMem = root.push(root.hMem, mem.value / 100)
            const d = Number(diskR.value) + Number(diskW.value)
            const nt = Number(netDown.value) + Number(netUp.value)
            root.peakDisk = Math.max(1048576, d, root.peakDisk * 0.98)
            root.peakNet = Math.max(131072, nt, root.peakNet * 0.98)
            root.hDisk = root.push(root.hDisk, d / root.peakDisk)
            root.hNet = root.push(root.hNet, nt / root.peakNet)
            for (let i = 0; i < gpuSensors.count; i++) {
                const g = gpuSensors.objectAt(i)
                if (g) {
                    g.hist = root.push(g.hist, g.usage.value / 100)
                    g.vhist = root.push(g.vhist, (Number(g.usedVram.value) || 0) / Math.max(1, Number(g.vramTotal.value) || 0))
                }
            }
            root.tick++
        }
    }

    function fmtRate(bytes) {
        bytes = Number(bytes) || 0
        const u = ["B", "K", "M", "G"]
        let i = 0
        while (bytes >= 1024 && i < 3) { bytes /= 1024; i++ }
        return (bytes < 10 && i > 0 ? bytes.toFixed(1) : bytes.toFixed(0)) + u[i]
    }
    // Same palette as the treetop app and the tray bars.
    readonly property var palette: ({ cpu: "#43c96b", mem: "#d6b23e", gpu: "#c06fd8", vram: "#d070a8", disk: "#4fb3d9", net: "#4f8fe0", temp: "#e0894f" })
    // CPU and main-GPU temperatures (°C), and a heat colour for them:
    // green up to 60°, yellow to 80°, red above.
    readonly property real cpuTempC: Number(cpuTemp.value) || 0
    readonly property real gpuTempC: {
        tick
        const i = gpuIndexes.length ? gpuIndexes[0] : (gpuPresent.length ? gpuPresent[0] : -1)
        const g = i >= 0 ? gpuSensors.objectAt(i) : null
        return g ? Number(g.temp.value) || 0 : 0
    }
    function tempColor(t) { return t >= 80 ? "#e05a4f" : (t >= 60 ? "#e3c25a" : "#43c96b") }
    function pct(v) { return (Number(v) || 0).toFixed(0) + "%" }
    // Label for an item per the "labels" setting: full name, one letter, or none.
    function lbl(full, letter) { return (cfg.labels === "full" || cfg.labels === "vertical" || cfg.labels === "stacked") ? full : (cfg.labels === "none" ? "" : letter) }
    readonly property bool vlabels: cfg.labels === "vertical"
    readonly property bool slabels: cfg.labels === "stacked"

    // ---- open treetop ----------------------------------------------------
    P5Support.DataSource {
        id: exec
        engine: "executable"
        connectedSources: []
        onNewData: (source) => disconnectSource(source)
    }
    // Show/hide treetop over D-Bus (works however it was started); if it
    // isn't running, start it (through devloop when that manages it).
    function toggleTreetop() {
        exec.connectSource("busctl --user call com.benwbooth.treetop /treetop com.benwbooth.treetop Toggle >/dev/null 2>&1"
            + " || devloop start treetop >/dev/null 2>&1"
            + " || setsid treetop >/dev/null 2>&1 &")
    }

    // ---- "who's using it" tooltips ---------------------------------------
    // Hovering an item asks the treetop app (over D-Bus) for the top
    // programs using that resource, refreshed while the pointer stays.
    property string hoverKind: ""
    property var topData: null
    function setHover(kind, on) {
        if (on) hoverKind = kind
        else if (hoverKind === kind) hoverKind = ""
    }
    P5Support.DataSource {
        id: topProbe
        engine: "executable"
        connectedSources: []
        onNewData: (source, data) => {
            try { root.topData = JSON.parse(JSON.parse(String(data.stdout)).data[0]) }
            catch (e) { root.topData = null }
            disconnectSource(source)
        }
    }
    Timer {
        interval: 1500; repeat: true; triggeredOnStart: true
        running: root.hoverKind !== ""
        onTriggered: topProbe.connectSource("busctl --user --json=short call com.benwbooth.treetop /treetop com.benwbooth.treetop Top")
    }
    function fmtTop(kind, v) {
        if (kind === "cpu" || kind === "gpu") return v.toFixed(v < 10 ? 1 : 0) + "%"
        if (kind === "mem" || kind === "vram") return fmtRate(v)
        return fmtRate(v) + "/s"
    }
    readonly property var kindTitle: ({ cpu: "CPU", mem: "Memory", gpu: "GPU", vram: "VRAM", disk: "Disk I/O", net: "Network", temp: "Temperatures & fans" })
    readonly property var kindColor: ({ cpu: palette.cpu, mem: palette.mem, gpu: palette.gpu, vram: palette.vram, disk: palette.disk, net: palette.net })
    // Every temperature and fan, grouped by device (from the treetop app).
    function sensorsText() {
        const list = topData && topData.sensors ? topData.sensors : null
        if (!list || !list.length) {
            let t = "<table cellspacing='1'>"
            if (cpuTempC > 0) t += "<tr><td><b>CPU</b></td><td align='right'>&nbsp;&nbsp;" + c(tempColor(cpuTempC), "<b>" + cpuTempC.toFixed(0) + "°C</b>") + "</td></tr>"
            if (gpuTempC > 0) t += "<tr><td><b>GPU</b></td><td align='right'>&nbsp;&nbsp;" + c(tempColor(gpuTempC), "<b>" + gpuTempC.toFixed(0) + "°C</b>") + "</td></tr>"
            return t + "</table><br>" + c(Kirigami.Theme.disabledTextColor, "Open treetop for every sensor and fan.")
        }
        let t = "<table cellspacing='1'>"
        let last = ""
        for (const x of list) {
            const v = x.kind === "fan"
                ? c(palette.net, "<b>" + x.value.toFixed(0) + " rpm</b>")
                : c(tempColor(x.value), "<b>" + x.value.toFixed(0) + "°C</b>")
            t += "<tr><td>" + (x.dev !== last ? "<b>" + esc(x.dev) + "</b>" : "") + "</td><td>&nbsp;&nbsp;"
                + c(Kirigami.Theme.disabledTextColor, esc(x.label)) + "</td><td align='right'>&nbsp;&nbsp;" + v + "</td></tr>"
            last = x.dev
        }
        return t + "</table>"
    }
    function topText(kind) {
        if (kind === "temp") return sensorsText()
        if (!topData || !topData[kind]) return c(Kirigami.Theme.disabledTextColor, "Open treetop to see which programs are using this.")
        const list = topData[kind]
        if (!list.length) return c(Kirigami.Theme.disabledTextColor, "Nothing is using it right now.")
        let t = "<table cellspacing='1'>"
        for (const x of list)
            t += "<tr><td align='right'>" + c(kindColor[kind], "<b>" + fmtTop(kind, x.v) + "</b>") + "</td><td>&nbsp;&nbsp;" + esc(x.name)
                + "</td><td>&nbsp;" + c(Kirigami.Theme.disabledTextColor, x.n > 1 ? "×" + x.n : "") + "</td></tr>"
        t += "</table>"
        if (kind === "net" || kind === "disk")
            t += c(Kirigami.Theme.disabledTextColor, "<br>Other users' processes need root access in treetop.")
        return t
    }

    toolTipMainText: !hoverKind ? "treetop" : (hoverKind === "temp" ? kindTitle.temp : kindTitle[hoverKind] + " — top programs")
    toolTipTextFormat: Text.RichText
    toolTipSubText: hoverKind ? topText(hoverKind) : summaryText
    // Our own tooltip content: Plasma's default one caps the number of
    // lines and clipped the lists; this sizes to whatever it shows.
    toolTipItem: Item {
        implicitWidth: tipCol.implicitWidth + Kirigami.Units.largeSpacing * 2
        implicitHeight: tipCol.implicitHeight + Kirigami.Units.largeSpacing * 2
        Layout.minimumWidth: implicitWidth
        Layout.minimumHeight: implicitHeight
        Layout.maximumWidth: implicitWidth
        Layout.maximumHeight: implicitHeight
        ColumnLayout {
            id: tipCol
            anchors.centerIn: parent
            spacing: Kirigami.Units.smallSpacing
            Kirigami.Heading {
                level: 4
                text: root.toolTipMainText
            }
            Text {
                textFormat: Text.RichText
                text: root.toolTipSubText
                color: Kirigami.Theme.textColor
                font.family: "monospace"
                font.pointSize: Kirigami.Theme.defaultFont.pointSize * 0.95
            }
        }
    }
    // Short GPU name: "Navi 31 [Radeon RX 7900 XT/...] (TUF Gaming Radeon RX
    // 7900 XTX OC)" -> "Radeon RX 7900 XTX"; "Raphael [Radeon 610M]" -> "Radeon 610M".
    function shortGpuName(n) {
        n = String(n || "")
        const paren = n.match(/\(([^)]*)\)\s*$/)
        if (paren) {
            const m = paren[1].match(/(Radeon|GeForce|Arc|Quadro|RTX|GTX|Intel)[^)]*/)
            if (m) return m[0].replace(/\s+(OC|Gaming|Edition)\b.*$/, "").trim()
        }
        const br = n.match(/\[([^\]]*)\]/)
        if (br) return br[1].split("/")[0].trim()
        return n.length > 28 ? n.slice(0, 27) + "…" : n
    }
    function esc(t) { return String(t).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;") }
    function c(color, t) { return "<font color='" + color + "'>" + t + "</font>" }
    function row(label, color, value, detail) {
        return "<tr><td>" + c(color, "<b>" + label + "</b>") + "</td><td align='right'>&nbsp;<b>" + value + "</b></td><td>&nbsp;&nbsp;"
            + c(Kirigami.Theme.disabledTextColor, detail || "") + "</td></tr>"
    }
    readonly property string summaryText: {
        tick
        let t = "<table cellspacing='1'>"
        t += row("CPU", palette.cpu, pct(cpu.value), cpuTemp.value ? Number(cpuTemp.value).toFixed(0) + "°C" : "")
        t += row("Memory", palette.mem, pct(mem.value), memUsed.formattedValue + " of " + memTotal.formattedValue)
        for (const i of root.gpuIndexes) {
            const g = gpuSensors.objectAt(i)
            if (!g) continue
            const n = root.gpuIndexes.length > 1 ? "GPU" + root.gpuNumber(i) : "GPU"
            t += row(n, palette.gpu, pct(g.usage.value), esc(shortGpuName(g.name.value)) + (g.temp.value ? " · " + Number(g.temp.value).toFixed(0) + "°C" : ""))
            t += row("VRAM", palette.vram, ((Number(g.usedVram.value) || 0) / Math.max(1, Number(g.vramTotal.value) || 0) * 100).toFixed(0) + "%", g.vram.formattedValue)
        }
        t += row("Disk", palette.disk, pct(diskUse.value), "read " + fmtRate(diskR.value) + "/s · write " + fmtRate(diskW.value) + "/s")
        t += row("Network", palette.net, "↓" + fmtRate(netDown.value), "↑" + fmtRate(netUp.value) + "/s")
        return t + "</table><br>" + c(Kirigami.Theme.disabledTextColor, "Click to open treetop · hover an item for its top users")
    }

    Plasmoid.contextualActions: [
        PlasmaCore.Action { text: "Open treetop"; icon.name: "utilities-system-monitor"; onTriggered: root.toggleTreetop() }
    ]

    // Plasma needs a full representation to build the compact one; this is
    // the popup if the widget is ever expanded (clicking opens treetop).
    switchWidth: Kirigami.Units.gridUnit * 10
    switchHeight: Kirigami.Units.gridUnit * 6
    fullRepresentation: ColumnLayout {
        Layout.minimumWidth: Kirigami.Units.gridUnit * 16
        Layout.minimumHeight: Kirigami.Units.gridUnit * 8
        Text {
            Layout.fillWidth: true
            text: root.toolTipSubText
            color: Kirigami.Theme.textColor
            font.family: "monospace"
            wrapMode: Text.Wrap
        }
    }

    // Inside the system tray the widget gets an icon-sized slot.
    readonly property bool inTray: Plasmoid.containment !== null
        && String(Plasmoid.containment.pluginName).indexOf("systemtray") >= 0

    compactRepresentation: MouseArea {
        id: compact
        readonly property bool vertical: Plasmoid.formFactor === PlasmaCore.Types.Vertical
        readonly property real h: vertical ? Math.min(width, Kirigami.Units.gridUnit * 1.6) : height
        Layout.minimumWidth: root.inTray || vertical ? -1 : strip.implicitWidth
        Layout.preferredWidth: root.inTray || vertical ? -1 : strip.implicitWidth
        Layout.minimumHeight: root.inTray || !vertical ? -1 : strip.implicitHeight
        Layout.preferredHeight: root.inTray || !vertical ? -1 : strip.implicitHeight
        hoverEnabled: true
        onClicked: root.toggleTreetop()

        // Tray look: five live bars (CPU, memory, GPU, VRAM, network).
        Row {
            visible: root.inTray
            anchors.centerIn: parent
            readonly property real s: Math.min(compact.width, compact.height)
            width: s
            height: s
            spacing: Math.max(1, s * 0.06)
            Repeater {
                model: 5
                Item {
                    readonly property color c: [root.palette.cpu, root.palette.mem, root.palette.gpu, root.palette.vram, root.palette.net][index]
                    readonly property real v: {
                        root.tick
                        if (index === 0) return cpu.value / 100
                        if (index === 1) return mem.value / 100
                        if (index === 2 || index === 3) {
                            // GPU1, the main card (most VRAM), like the panel strip.
                            const i = root.gpuIndexes.length ? root.gpuIndexes[0] : (root.gpuPresent.length ? root.gpuPresent[0] : -1)
                            const g = i >= 0 ? gpuSensors.objectAt(i) : null
                            if (!g) return 0
                            return index === 2 ? g.usage.value / 100
                                : (Number(g.usedVram.value) || 0) / Math.max(1, Number(g.vramTotal.value) || 0)
                        }
                        return (Number(netDown.value) + Number(netUp.value)) / root.peakNet
                    }
                    width: (parent.s - parent.spacing * 4) / 5
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
                        color: parent.c
                        height: Math.max(1, Math.min(1, parent.v || 0) * parent.height)
                    }
                }
            }
        }

        // Panel look: KStats-style strip.
        GridLayout {
            id: strip
            visible: !root.inTray
            anchors.fill: parent
            flow: compact.vertical ? GridLayout.TopToBottom : GridLayout.LeftToRight
            rows: compact.vertical ? -1 : 1
            columns: compact.vertical ? 1 : -1
            columnSpacing: Math.round(compact.h * 0.16)
            rowSpacing: 2
            readonly property real gw: root.cfg.graphWidth / 100

            KStat {
                visible: root.cfg.showCpu
                h: compact.h; label: root.lbl("CPU", "C"); color: root.palette.cpu
                onHoverChanged: (on) => root.setHover("cpu", on)
                value: (Number(cpu.value) || 0).toFixed(0) + "%"
                history: root.hCpu
                showGraph: root.cfg.showGraphs; colorLabel: root.cfg.colorLabels; graphWidthFactor: strip.gw; overlay: root.cfg.valuesInside; verticalLabel: root.vlabels; stackedLabel: root.slabels
            }
            KStat {
                visible: root.cfg.showMemory
                h: compact.h; label: root.lbl("MEM", "M"); color: root.palette.mem
                onHoverChanged: (on) => root.setHover("mem", on)
                value: (Number(mem.value) || 0).toFixed(0) + "%"
                history: root.hMem
                showGraph: root.cfg.showGraphs; colorLabel: root.cfg.colorLabels; graphWidthFactor: strip.gw; overlay: root.cfg.valuesInside; verticalLabel: root.vlabels; stackedLabel: root.slabels
            }
            Repeater {
                model: root.gpuIndexes
                RowLayout {
                    required property int modelData
                    readonly property var g: { root.tick; return gpuSensors.objectAt(modelData) }
                    readonly property string n: root.gpuIndexes.length > 1 ? String(root.gpuNumber(modelData)) : ""
                    spacing: Math.round(compact.h * 0.16)
                    KStat {
                        h: compact.h
                        label: root.lbl("GPU", "G") + parent.n
                        onHoverChanged: (on) => root.setHover("gpu", on)
                        color: root.palette.gpu
                        value: parent.g ? (Number(parent.g.usage.value) || 0).toFixed(0) + "%" : "–"
                        history: parent.g ? parent.g.hist : []
                        showGraph: root.cfg.showGraphs; colorLabel: root.cfg.colorLabels; graphWidthFactor: strip.gw; overlay: root.cfg.valuesInside; verticalLabel: root.vlabels; stackedLabel: root.slabels
                    }
                    // GPU memory: history of VRAM use, amount used inside.
                    KStat {
                        visible: root.cfg.showVram
                        h: compact.h
                        label: root.lbl("VRAM", "V") + parent.n
                        onHoverChanged: (on) => root.setHover("vram", on)
                        color: root.palette.vram
                        value: parent.g ? ((Number(parent.g.usedVram.value) || 0) / Math.max(1, Number(parent.g.vramTotal.value) || 0) * 100).toFixed(0) + "%" : "–"
                        history: parent.g ? parent.g.vhist : []
                        showGraph: root.cfg.showGraphs; colorLabel: root.cfg.colorLabels; graphWidthFactor: strip.gw; overlay: root.cfg.valuesInside; verticalLabel: root.vlabels; stackedLabel: root.slabels
                    }
                }
            }
            KStat {
                visible: root.cfg.showDisk
                h: compact.h; label: root.lbl("DISK", "D"); color: root.palette.disk
                onHoverChanged: (on) => root.setHover("disk", on)
                readonly property bool io: root.cfg.diskMode === "io"
                value: io ? root.fmtRate(Number(diskR.value) + Number(diskW.value)) : (Number(diskUse.value) || 0).toFixed(0) + "%"
                bar: !io
                fill: (Number(diskUse.value) || 0) / 100
                history: root.hDisk
                showGraph: io && root.cfg.showGraphs; colorLabel: root.cfg.colorLabels; graphWidthFactor: strip.gw; overlay: root.cfg.valuesInside; verticalLabel: root.vlabels; stackedLabel: root.slabels
            }
            // Network: arrows and two stacked rates, like KStats.
            RowLayout {
                visible: root.cfg.showNetwork
                HoverHandler { onHoveredChanged: root.setHover("net", hovered) }
                spacing: Math.round(compact.h * 0.08)
                Item {
                    visible: root.lbl("NET", "N").length > 0
                    Layout.alignment: Qt.AlignVCenter
                    implicitWidth: root.slabels ? netStacked.implicitWidth : (root.vlabels ? netLabel.implicitHeight : netLabel.implicitWidth)
                    implicitHeight: root.slabels ? netStacked.implicitHeight : (root.vlabels ? netLabel.implicitWidth : netLabel.implicitHeight)
                    StackedLabel {
                        id: netStacked
                        visible: root.slabels
                        anchors.centerIn: parent
                        text: "NET"
                        h: compact.h
                        color: root.cfg.colorLabels ? root.palette.net : Kirigami.Theme.textColor
                    }
                    Text {
                        id: netLabel
                        visible: !root.slabels
                        anchors.centerIn: parent
                        rotation: root.vlabels ? -90 : 0
                        text: root.lbl("NET", "N")
                        horizontalAlignment: Text.AlignHCenter
                        lineHeightMode: root.slabels ? Text.FixedHeight : Text.ProportionalHeight
                        lineHeight: root.slabels ? Math.floor(compact.h * 0.92 / 4) : 1
                        color: root.cfg.colorLabels ? root.palette.net : Kirigami.Theme.textColor
                        font.pixelSize: root.slabels ? Math.max(5, Math.floor(compact.h * 0.92 / 4 * 0.8))
                            : (root.vlabels ? Math.max(6, Math.round(compact.h * 0.2)) : Math.max(7, Math.round(compact.h * 0.22)))
                        font.bold: true
                        renderType: Text.NativeRendering
                    }
                }
                Column {
                    spacing: -2
                    Text { text: "↑" + root.fmtRate(netUp.value); color: Kirigami.Theme.textColor; font.family: "monospace"; font.pixelSize: Math.round(compact.h * 0.3); renderType: Text.NativeRendering }
                    Text { text: "↓" + root.fmtRate(netDown.value); color: Kirigami.Theme.textColor; font.family: "monospace"; font.pixelSize: Math.round(compact.h * 0.3); renderType: Text.NativeRendering }
                }
            }
            // Temperatures: CPU over GPU, each coloured by heat.
            RowLayout {
                visible: root.cfg.showTemps && (root.cpuTempC > 0 || root.gpuTempC > 0)
                spacing: Math.round(compact.h * 0.08)
                HoverHandler { onHoveredChanged: root.setHover("temp", hovered) }
                Item {
                    visible: root.lbl("TEMP", "T").length > 0
                    Layout.alignment: Qt.AlignVCenter
                    implicitWidth: root.slabels ? tempStacked.implicitWidth : (root.vlabels ? tempLabel.implicitHeight : tempLabel.implicitWidth)
                    implicitHeight: root.slabels ? tempStacked.implicitHeight : (root.vlabels ? tempLabel.implicitWidth : tempLabel.implicitHeight)
                    StackedLabel {
                        id: tempStacked
                        visible: root.slabels
                        anchors.centerIn: parent
                        text: "TEMP"
                        h: compact.h
                        color: root.cfg.colorLabels ? root.palette.temp : Kirigami.Theme.textColor
                    }
                    Text {
                        id: tempLabel
                        visible: !root.slabels
                        anchors.centerIn: parent
                        rotation: root.vlabels ? -90 : 0
                        text: root.lbl("TEMP", "T")
                        color: root.cfg.colorLabels ? root.palette.temp : Kirigami.Theme.textColor
                        font.pixelSize: root.vlabels ? Math.max(6, Math.round(compact.h * 0.2)) : Math.max(7, Math.round(compact.h * 0.22))
                        font.bold: true
                        renderType: Text.NativeRendering
                    }
                }
                Column {
                    spacing: -2
                    Text {
                        visible: root.cpuTempC > 0
                        text: "CPU " + root.cpuTempC.toFixed(0) + "°"
                        color: root.tempColor(root.cpuTempC)
                        font.family: "monospace"; font.pixelSize: Math.round(compact.h * 0.3); font.bold: true
                        renderType: Text.NativeRendering
                    }
                    Text {
                        visible: root.gpuTempC > 0
                        text: "GPU " + root.gpuTempC.toFixed(0) + "°"
                        color: root.tempColor(root.gpuTempC)
                        font.family: "monospace"; font.pixelSize: Math.round(compact.h * 0.3); font.bold: true
                        renderType: Text.NativeRendering
                    }
                }
            }
        }
    }
}
