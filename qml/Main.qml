import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtCore as QtCore
import Qt.labs.platform as Platform
import com.benwbooth.treetop
import "Fmt.js" as Fmt

ApplicationWindow {
    id: win
    property bool startHidden: false
    visible: !startHidden
    width: 1280
    height: 860
    minimumWidth: 720
    minimumHeight: 480
    title: "treetop"

    // ---- theme -----------------------------------------------------------
    SystemPalette { id: pal; colorGroup: SystemPalette.Active }
    readonly property bool dark: pal.window.hslLightness < 0.5
    readonly property color textColor: pal.windowText
    readonly property color dimColor: Qt.rgba(pal.windowText.r, pal.windowText.g, pal.windowText.b, 0.55)
    readonly property color trackColor: dark ? Qt.rgba(1, 1, 1, 0.07) : Qt.rgba(0, 0, 0, 0.07)
    readonly property font mono: Qt.font({
        family: typeof systemFixedFont !== "undefined" ? systemFixedFont.family : "monospace",
        styleHint: Font.Monospace,
        pointSize: Math.max(6, (typeof systemFixedFont !== "undefined" ? systemFixedFont.pointSize : 10) - 1.5 + settings.fontDelta),
    })
    // htop's palette, nudged for light themes.
    readonly property color cGreen: dark ? "#43c96b" : "#1f9d48"
    readonly property color cBlue: dark ? "#4f8fe0" : "#2a64c4"
    readonly property color cRed: dark ? "#e05a4f" : "#c63a2f"
    readonly property color cYellow: dark ? "#d6b23e" : "#b58a10"
    readonly property color cCyan: dark ? "#4fb3d9" : "#1b84ad"
    readonly property color cMagenta: dark ? "#c06fd8" : "#9440ae"
    readonly property color cOrange: dark ? "#e0894f" : "#c2621f"
    color: pal.window

    // ---- state -----------------------------------------------------------
    property var snap: ({ cpu: { cores: [] }, mem: {}, gpus: [], disk: { filesystems: [] }, net: {}, load: [0, 0, 0], procs: {} })
    readonly property int historyLength: 90
    property var hist: ({ cpu: [], mem: [], rx: [], tx: [], rd: [], wr: [], gpu: [], vram: [] })
    // GPUs the user hid (by PCI address), and the rest with their numbers.
    readonly property var hiddenGpus: settings.hiddenGpus.split(",").filter(x => x.length > 0)
    readonly property var gpus: snap.gpus.map((g, i) => Object.assign({ number: i + 1, index: i }, g)).filter(g => hiddenGpus.indexOf(g.pdev) < 0)
    function hideGpu(pdev) {
        const h = hiddenGpus.slice(); if (h.indexOf(pdev) < 0) h.push(pdev)
        settings.hiddenGpus = h.join(",")
    }
    property string status: ""
    property string view: settings.view

    QtCore.Settings {
        id: settings
        property int interval: 1000
        property string view: "tree"
        property bool showMeters: true
        property bool showCards: true
        property bool showKernel: false
        property real fontDelta: 0
        property string hiddenColumns: "pri,ni,shr,gpus,engines,container"
        property bool shortenStorePaths: true
        property bool showResources: true
        property bool cpuCombined: false
        property string hiddenGpus: ""
        property alias winWidth: win.width
        property alias winHeight: win.height
    }

    Monitor {
        id: mon
        onUpdated: (json) => win.apply(JSON.parse(json))
    }
    onVisibleChanged: mon.set_rows_enabled(visible)
    Connections {
        target: settings
        function onIntervalChanged() { mon.set_interval(settings.interval) }
    }

    function push(arr, v) {
        const a = arr.slice(arr.length >= historyLength ? 1 : 0)
        a.push(Number(v) || 0)
        return a
    }

    // Called for every update the sampler thread posts.
    function apply(s) {
        if (!s.rowsOnly) {
            const h = hist
            const g = h.gpu.slice(), vr = h.vram.slice()
            for (let i = 0; i < s.gpus.length; i++) {
                g[i] = push(g[i] || [], s.gpus[i].busy)
                vr[i] = push(vr[i] || [], s.gpus[i].vramUsed / Math.max(1, s.gpus[i].vramTotal) * 100)
            }
            hist = {
                cpu: push(h.cpu, s.cpu.total),
                mem: push(h.mem, s.mem.used / s.mem.total * 100),
                rx: push(h.rx, s.net.rx), tx: push(h.tx, s.net.tx),
                rd: push(h.rd, s.disk.read), wr: push(h.wr, s.disk.write),
                gpu: g, vram: vr,
            }
            snap = s
            for (const n of s.notices) showStatus(n)
        }
        if (s.rows) table.applyDiff(s.rows)
    }

    function showStatus(text) {
        if (!text) return
        status = text
        statusTimer.restart()
    }
    Timer { id: statusTimer; interval: 6000; onTriggered: win.status = "" }

    Component.onCompleted: {
        mon.set_interval(settings.interval)
        mon.set_rows_enabled(win.visible)
        mon.set_view(settings.view)
        mon.set_show_kernel(settings.showKernel)
        mon.set_sort(table.sortKey, table.sortDesc)
    }

    function setView(v) {
        settings.view = v
        mon.set_view(v)
        if (v === "gpu" && table.sortKey !== "gpu") { table.sortKey = ""; table.sortBy("gpu") }
    }

    // ---- window/tray plumbing (called from C++ for single-instance) -----
    function showWindow() {
        win.show()
        win.raise()
        win.requestActivate()
    }
    function toggleWindow() {
        if (win.visible && win.active) win.hide(); else showWindow()
    }
    onClosing: (close) => {
        if (tray.available) { close.accepted = false; win.hide() }
    }

    readonly property string summary: {
        const s = snap
        if (!s.mem || !s.mem.total) return "treetop"
        let t = "CPU " + Fmt.pct(s.cpu.total) + " · MEM " + Fmt.pct(s.mem.used / s.mem.total * 100)
        for (const gp of gpus) t += " · GPU" + gp.number + " " + gp.busy.toFixed(0) + "%"
        t += "\n↓ " + Fmt.rate(s.net.rx) + "  ↑ " + Fmt.rate(s.net.tx)
        t += "\nDisk R " + Fmt.rate(s.disk.read) + "  W " + Fmt.rate(s.disk.write)
        return t
    }

    Platform.SystemTrayIcon {
        id: tray
        visible: available
        icon.source: Qt.resolvedUrl("treetop.svg")
        tooltip: win.summary
        onActivated: (reason) => {
            if (reason === Platform.SystemTrayIcon.Trigger || reason === Platform.SystemTrayIcon.Unknown)
                Qt.callLater(win.toggleWindow)
        }
        menu: Platform.Menu {
            Platform.MenuItem { text: win.visible ? "Hide treetop" : "Show treetop"; onTriggered: Qt.callLater(win.toggleWindow) }
            Platform.MenuSeparator {}
            Platform.MenuItem { text: "Process tree"; checkable: true; checked: win.view === "tree"; onTriggered: { win.setView("tree"); win.showWindow() } }
            Platform.MenuItem { text: "Containers"; checkable: true; checked: win.view === "containers"; onTriggered: { win.setView("containers"); win.showWindow() } }
            Platform.MenuItem { text: "GPU"; checkable: true; checked: win.view === "gpu"; onTriggered: { win.setView("gpu"); win.showWindow() } }
            Platform.MenuSeparator {}
            Platform.MenuItem { text: "Quit"; onTriggered: Qt.quit() }
        }
    }

    // ---- shortcuts (htop keys) ------------------------------------------
    Shortcut { sequences: ["F3", "Ctrl+F"]; onActivated: filterField.forceActiveFocus() }
    Shortcut { sequence: "F5"; onActivated: win.setView(win.view === "tree" ? "flat" : "tree") }
    Shortcut { sequence: "F10"; onActivated: win.hide() }
    Shortcut { sequence: "Ctrl+Q"; onActivated: Qt.quit() }
    Shortcut { sequence: "Escape"; onActivated: { if (filterField.text) { filterField.text = ""; table.forceActiveFocus() } else win.hide() } }
    // Single-letter htop keys only act while the process list has focus,
    // so typing in the filter box doesn't trigger them.
    function letterKey(t) {
        switch (t) {
        case "/": filterField.forceActiveFocus(); return true
        case "t": win.setView(win.view === "tree" ? "flat" : "tree"); return true
        case "c": win.setView(win.view === "containers" ? "tree" : "containers"); return true
        case "K": settings.showKernel = !settings.showKernel; mon.set_show_kernel(settings.showKernel); table.refreshRows(); return true
        case "P": table.sortKey = "pid"; table.sortBy("cpu"); return true
        case "M": table.sortKey = "cpu"; table.sortBy("mem"); return true
        case "T": table.sortKey = "cpu"; table.sortBy("time"); return true
        case "g": win.setView(win.view === "gpu" ? "tree" : "gpu"); return true
        case "1": settings.cpuCombined = !settings.cpuCombined; return true
        case "r": settings.showResources = !settings.showResources; return true
        case "q": win.hide(); return true
        case "h": case "?": helpDialog.open(); return true
        }
        return false
    }
    Shortcut { sequence: "F1"; onActivated: helpDialog.open() }
    Shortcut { sequences: ["Ctrl+=", "Ctrl++"]; onActivated: settings.fontDelta = Math.min(8, settings.fontDelta + 0.5) }
    Shortcut { sequence: "Ctrl+-"; onActivated: settings.fontDelta = Math.max(-4, settings.fontDelta - 0.5) }
    Shortcut { sequence: "Ctrl+0"; onActivated: settings.fontDelta = 0 }

    // ---- layout ----------------------------------------------------------
    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // Toolbar
        ToolBar {
            Layout.fillWidth: true
            RowLayout {
                anchors.fill: parent
                spacing: 6
                ButtonGroup { id: viewGroup }
                ToolButton { text: "Tree"; checkable: true; checked: win.view === "tree"; ButtonGroup.group: viewGroup; onClicked: win.setView("tree"); icon.name: "view-list-tree" }
                ToolButton { text: "List"; checkable: true; checked: win.view === "flat"; ButtonGroup.group: viewGroup; onClicked: win.setView("flat"); icon.name: "view-list-details" }
                ToolButton { text: "Containers"; checkable: true; checked: win.view === "containers"; ButtonGroup.group: viewGroup; onClicked: win.setView("containers"); icon.name: "application-x-docker" }
                ToolButton { text: "GPU"; checkable: true; checked: win.view === "gpu"; ButtonGroup.group: viewGroup; onClicked: win.setView("gpu"); icon.name: "video-display" }
                ToolSeparator {}
                TextField {
                    id: filterField
                    Layout.preferredWidth: 280
                    placeholderText: "Filter: name, command, user, container, PID  (/)"
                    onTextChanged: mon.set_filter(text)
                    Keys.onDownPressed: table.forceActiveFocus()
                    Keys.onReturnPressed: table.forceActiveFocus()
                }
                CheckBox { text: "Kernel threads"; checked: settings.showKernel; onToggled: { settings.showKernel = checked; mon.set_show_kernel(checked); table.refreshRows() } }
                Item { Layout.fillWidth: true }
                Label { text: "Update"; color: win.dimColor }
                ComboBox {
                    model: [{ t: "0.25 s", v: 250 }, { t: "0.5 s", v: 500 }, { t: "1 s", v: 1000 }, { t: "2 s", v: 2000 }, { t: "5 s", v: 5000 }]
                    textRole: "t"
                    valueRole: "v"
                    Component.onCompleted: currentIndex = indexOfValue(settings.interval)
                    onActivated: settings.interval = currentValue
                }
                ToolButton { icon.name: "view-statistics"; checkable: true; checked: settings.showCards; onToggled: settings.showCards = checked; ToolTip.text: "Graphs"; ToolTip.visible: hovered }
                ToolButton { icon.name: "view-process-system"; checkable: true; checked: settings.showMeters; onToggled: settings.showMeters = checked; ToolTip.text: "Meters"; ToolTip.visible: hovered }
                ToolButton { icon.name: "help-contents"; onClicked: helpDialog.open(); ToolTip.text: "Keys (h)"; ToolTip.visible: hovered }
            }
        }

        // Collapsible resources section
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: resHead.implicitHeight + 6
            color: resHover.hovered ? Qt.rgba(win.textColor.r, win.textColor.g, win.textColor.b, 0.06) : "transparent"
            HoverHandler { id: resHover }
            TapHandler { onTapped: settings.showResources = !settings.showResources }
            TapHandler { acceptedButtons: Qt.RightButton; onTapped: resMenu.popup() }
            RowLayout {
                id: resHead
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 8
                Text { text: (settings.showResources ? "▾ " : "▸ ") + "Resources"; font.family: win.mono.family; font.pointSize: win.mono.pointSize; font.bold: true; color: win.textColor }
                Text {
                    // A one-line summary, mostly useful when collapsed.
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                    font: win.mono
                    color: win.dimColor
                    text: win.summary.replace(/\n/g, "  ·  ")
                }
                Text { text: "r"; font: win.mono; color: win.dimColor }
            }
            Menu {
                id: resMenu
                MenuItem { text: "Graphs"; checkable: true; checked: settings.showCards; onTriggered: settings.showCards = !settings.showCards }
                MenuItem { text: "Meters"; checkable: true; checked: settings.showMeters; onTriggered: settings.showMeters = !settings.showMeters }
                MenuItem { text: "Combine CPU cores (1)"; checkable: true; checked: settings.cpuCombined; onTriggered: settings.cpuCombined = !settings.cpuCombined }
                MenuSeparator {}
                Repeater {
                    model: win.snap.gpus
                    MenuItem {
                        text: "Show GPU" + (index + 1) + " — " + modelData.name
                        checkable: true
                        checked: win.hiddenGpus.indexOf(modelData.pdev) < 0
                        onTriggered: {
                            if (checked) settings.hiddenGpus = win.hiddenGpus.filter(p => p !== modelData.pdev).join(",")
                            else win.hideGpu(modelData.pdev)
                        }
                    }
                }
            }
        }

        // kstats half: graph cards
        RowLayout {
            visible: settings.showResources && settings.showCards
            Layout.fillWidth: true
            Layout.margins: 8
            Layout.bottomMargin: 0
            spacing: 8
            StatCard {
                Layout.fillWidth: true
                title: "CPU"; accent: win.cGreen; textColor: win.textColor; dimColor: win.dimColor; font: win.mono
                value: Fmt.pct(win.snap.cpu.total || 0)
                detail: (win.snap.cpu.temp ? win.snap.cpu.temp.toFixed(0) + "°C · " : "") + "load " + win.snap.load.map(v => v.toFixed(2)).join(" ")
                series: [{ values: win.hist.cpu, color: win.cGreen }]
                maximum: 100
            }
            StatCard {
                Layout.fillWidth: true
                title: "Memory"; accent: win.cYellow; textColor: win.textColor; dimColor: win.dimColor; font: win.mono
                value: win.snap.mem.total ? Fmt.pct(win.snap.mem.used / win.snap.mem.total * 100) : ""
                detail: Fmt.bytes(win.snap.mem.used) + " / " + Fmt.bytes(win.snap.mem.total) + (win.snap.mem.swapUsed > 0 ? " · swap " + Fmt.bytes(win.snap.mem.swapUsed) : "")
                series: [{ values: win.hist.mem, color: win.cYellow }]
                maximum: 100
            }
            Repeater {
                model: win.gpus
                StatCard {
                    Layout.fillWidth: true
                    title: "GPU " + modelData.number; accent: win.cMagenta; textColor: win.textColor; dimColor: win.dimColor; font: win.mono
                    value: modelData.busy.toFixed(0) + "%"
                    detail: Fmt.bytes(modelData.vramUsed) + " / " + Fmt.bytes(modelData.vramTotal)
                        + (modelData.temp ? " · " + modelData.temp.toFixed(0) + "°C" : "")
                        + (modelData.power ? " · " + modelData.power.toFixed(0) + "W" : "")
                    series: [{ values: win.hist.gpu[modelData.index] || [], color: win.cMagenta }]
                    maximum: 100
                    ToolTip.text: modelData.name + "\nClick for the GPU view · right-click to hide"
                    ToolTip.visible: gpuHover.hovered
                    ToolTip.delay: 600
                    HoverHandler { id: gpuHover }
                    onClicked: win.setView(win.view === "gpu" ? "tree" : "gpu")
                    TapHandler { acceptedButtons: Qt.RightButton; onTapped: gpuCardMenu.popup() }
                    Menu {
                        id: gpuCardMenu
                        MenuItem { text: "Hide GPU " + modelData.number; onTriggered: win.hideGpu(modelData.pdev) }
                        MenuItem { text: "GPU view"; onTriggered: win.setView("gpu") }
                    }
                }
            }
            StatCard {
                Layout.fillWidth: true
                title: "Disk"; accent: win.cCyan; textColor: win.textColor; dimColor: win.dimColor; font: win.mono
                value: Fmt.rate((win.snap.disk.read || 0) + (win.snap.disk.write || 0))
                detail: "R " + Fmt.rate(win.snap.disk.read) + " · W " + Fmt.rate(win.snap.disk.write)
                series: [{ values: win.hist.rd, color: win.cCyan }, { values: win.hist.wr, color: win.cOrange }]
                mirrored: true
            }
            StatCard {
                Layout.fillWidth: true
                title: "Network"; accent: win.cBlue; textColor: win.textColor; dimColor: win.dimColor; font: win.mono
                value: Fmt.rate((win.snap.net.rx || 0) + (win.snap.net.tx || 0))
                detail: "↓ " + Fmt.rate(win.snap.net.rx) + " · ↑ " + Fmt.rate(win.snap.net.tx)
                series: [{ values: win.hist.rx, color: win.cBlue }, { values: win.hist.tx, color: win.cRed }]
                mirrored: true
            }
        }

        // GPU view: amdgpu_top-style panels
        ColumnLayout {
            visible: settings.showResources && win.view === "gpu"
            Layout.fillWidth: true
            Layout.margins: 8
            Layout.bottomMargin: 4
            spacing: 6
            Repeater {
                model: win.gpus
                GpuPanel {
                    Layout.fillWidth: true
                    gpu: modelData
                    number: modelData.number
                    history: win.hist.gpu[modelData.index] || []
                    vramHistory: win.hist.vram[modelData.index] || []
                    font: win.mono; textColor: win.textColor; dimColor: win.dimColor; trackColor: win.trackColor
                    accent: win.cMagenta; memColor: win.cCyan
                    onHideRequested: win.hideGpu(modelData.pdev)
                }
            }
            Text {
                visible: win.gpus.length === 0
                text: "No GPUs shown (right-click the Resources bar to unhide)."
                font: win.mono; color: win.dimColor
            }
        }

        // htop half: meters
        RowLayout {
            visible: settings.showResources && settings.showMeters && win.view !== "gpu"
            Layout.fillWidth: true
            Layout.margins: 8
            Layout.bottomMargin: 4
            spacing: 16

            ColumnLayout {
                visible: settings.cpuCombined
                Layout.fillWidth: true
                Layout.preferredWidth: win.width * 0.55
                Layout.alignment: Qt.AlignTop
                spacing: 1
                Meter {
                    Layout.fillWidth: true
                    font: win.mono; textColor: win.textColor; trackColor: win.trackColor
                    label: "CPU"
                    labelWidth: 5 * win.mono.pointSize
                    segments: [
                        { value: win.snap.cpu.nice || 0, color: win.cBlue },
                        { value: win.snap.cpu.user || 0, color: win.cGreen },
                        { value: win.snap.cpu.system || 0, color: win.cRed },
                        { value: win.snap.cpu.other || 0, color: win.cOrange },
                    ]
                    text: Fmt.pct(win.snap.cpu.total || 0) + "  ·  " + win.snap.cpu.cores.length + " threads"
                        + (win.snap.cpu.temp ? "  ·  " + win.snap.cpu.temp.toFixed(0) + "°C" : "")
                }
                Text {
                    font: win.mono; color: win.dimColor
                    text: "user " + Fmt.pct(win.snap.cpu.user) + "  sys " + Fmt.pct(win.snap.cpu.system) + "  nice " + Fmt.pct(win.snap.cpu.nice) + "  irq/io " + Fmt.pct(win.snap.cpu.other)
                        + "   (click or press 1 for per-core)"
                }
                TapHandler { onTapped: settings.cpuCombined = false }
            }
            GridLayout {
                id: coreGrid
                visible: !settings.cpuCombined
                Layout.fillWidth: true
                Layout.preferredWidth: win.width * 0.55
                Layout.alignment: Qt.AlignTop
                TapHandler { onTapped: settings.cpuCombined = true }
                HoverHandler { id: coreHover }
                ToolTip.text: "Per-core usage: green user, blue nice, red system, orange irq/iowait/steal. Click (or press 1) to combine into one meter."
                ToolTip.visible: coreHover.hovered
                ToolTip.delay: 1200
                readonly property int n: win.snap.cpu.cores.length
                columns: n > 16 ? 4 : (n > 8 ? 2 : 1)
                columnSpacing: 12
                rowSpacing: 1
                flow: GridLayout.TopToBottom
                rows: Math.ceil(n / columns)
                Repeater {
                    model: win.snap.cpu.cores
                    Meter {
                        Layout.fillWidth: true
                        Layout.minimumWidth: 140
                        font: win.mono; textColor: win.textColor; trackColor: win.trackColor
                        label: String(index)
                        labelWidth: 3.5 * win.mono.pointSize
                        segments: [
                            { value: modelData[1], color: win.cBlue },
                            { value: modelData[0], color: win.cGreen },
                            { value: modelData[2], color: win.cRed },
                            { value: modelData[3], color: win.cOrange },
                        ]
                        text: Fmt.pct(modelData[0] + modelData[1] + modelData[2] + modelData[3])
                    }
                }
            }

            ColumnLayout {
                Layout.preferredWidth: Math.max(320, win.width * 0.38)
                Layout.alignment: Qt.AlignTop
                spacing: 1
                Meter {
                    Layout.fillWidth: true
                    font: win.mono; textColor: win.textColor; trackColor: win.trackColor
                    label: "Mem"
                    labelWidth: 9 * win.mono.pointSize
                    maximum: win.snap.mem.total || 1
                    segments: [
                        { value: win.snap.mem.used, color: win.cGreen },
                        { value: win.snap.mem.buffers, color: win.cBlue },
                        { value: win.snap.mem.cached, color: win.cYellow },
                    ]
                    text: Fmt.bytes(win.snap.mem.used) + "/" + Fmt.bytes(win.snap.mem.total)
                }
                Meter {
                    Layout.fillWidth: true
                    visible: (win.snap.mem.swapTotal || 0) > 0
                    font: win.mono; textColor: win.textColor; trackColor: win.trackColor
                    label: "Swp"
                    labelWidth: 9 * win.mono.pointSize
                    maximum: win.snap.mem.swapTotal || 1
                    segments: [{ value: win.snap.mem.swapUsed, color: win.cRed }]
                    text: Fmt.bytes(win.snap.mem.swapUsed) + "/" + Fmt.bytes(win.snap.mem.swapTotal)
                }
                Repeater {
                    model: win.gpus
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 1
                        TapHandler { acceptedButtons: Qt.RightButton; onTapped: gpuMeterMenu.popup() }
                        TapHandler { onTapped: win.setView("gpu") }
                        Menu {
                            id: gpuMeterMenu
                            MenuItem { text: "Hide GPU " + modelData.number; onTriggered: win.hideGpu(modelData.pdev) }
                            MenuItem { text: "GPU view"; onTriggered: win.setView("gpu") }
                        }
                        Meter {
                            Layout.fillWidth: true
                            font: win.mono; textColor: win.textColor; trackColor: win.trackColor
                            label: "GPU" + modelData.number
                            labelWidth: 9 * win.mono.pointSize
                            segments: [{ value: modelData.busy, color: win.cMagenta }]
                            text: modelData.busy.toFixed(0) + "%" + (modelData.temp ? "  " + modelData.temp.toFixed(0) + "°C" : "") + (modelData.power ? "  " + modelData.power.toFixed(0) + "W" : "")
                        }
                        Meter {
                            Layout.fillWidth: true
                            font: win.mono; textColor: win.textColor; trackColor: win.trackColor
                            label: "VRAM"
                            labelWidth: 9 * win.mono.pointSize
                            maximum: modelData.vramTotal || 1
                            segments: [{ value: modelData.vramUsed, color: win.cCyan }]
                            text: Fmt.bytes(modelData.vramUsed) + "/" + Fmt.bytes(modelData.vramTotal)
                        }
                    }
                }
                Repeater {
                    model: win.snap.disk.filesystems
                    Meter {
                        Layout.fillWidth: true
                        font: win.mono; textColor: win.textColor; trackColor: win.trackColor
                        label: modelData.mount
                        labelWidth: 9 * win.mono.pointSize
                        maximum: modelData.total
                        segments: [{ value: modelData.used, color: modelData.used / modelData.total > 0.9 ? win.cRed : win.cCyan }]
                        text: Fmt.bytes(modelData.used) + "/" + Fmt.bytes(modelData.total)
                        ToolTip.text: modelData.device + " (" + modelData.type + ")"
                        ToolTip.visible: fsHover.hovered
                        HoverHandler { id: fsHover }
                    }
                }
                Text {
                    Layout.topMargin: 3
                    font: win.mono
                    color: win.textColor
                    textFormat: Text.StyledText
                    readonly property var p: win.snap.procs
                    text: "<font color='" + win.cCyan + "'>Tasks:</font> <b>" + (p.tasks || 0) + "</b>, " + (p.threads || 0) + " thr, "
                        + (p.kthreads || 0) + " kthr; <font color='" + win.cGreen + "'><b>" + (p.running || 0) + "</b> running</font>"
                        + (p.containers ? "; <font color='" + win.cMagenta + "'><b>" + p.containers + "</b> containers</font>" : "")
                }
                Text {
                    font: win.mono
                    color: win.textColor
                    textFormat: Text.StyledText
                    text: "<font color='" + win.cCyan + "'>Load average:</font> <b>" + win.snap.load.map(v => v.toFixed(2)).join(" ") + "</b>"
                        + "   <font color='" + win.cCyan + "'>Uptime:</font> <b>" + Fmt.uptime(win.snap.uptime) + "</b>"
                }
                Text {
                    font: win.mono
                    color: win.textColor
                    textFormat: Text.StyledText
                    text: {
                        let s = "<font color='" + win.cCyan + "'>Net:</font> "
                        const ifs = (win.snap.net.interfaces || []).filter(i => i.physical)
                        s += ifs.map(i => "<b>" + i.name + "</b> ↓" + Fmt.rate(i.rx) + " ↑" + Fmt.rate(i.tx)).join("  ")
                        return s
                    }
                }
            }
        }

        // The process table
        ProcessTable {
            id: table
            Layout.fillWidth: true
            Layout.fillHeight: true
            monitor: mon
            view: win.view
            focus: true
            font: win.mono
            textColor: win.textColor
            dimColor: win.dimColor
            baseColor: pal.base
            altColor: pal.alternateBase
            highlightColor: pal.highlight
            highlightTextColor: pal.highlightedText
            showContainerColumn: win.view !== "containers" && (win.snap.procs.containers || 0) > 0
            onNotice: (text) => win.showStatus(text)
            onShowDetails: (pid) => details.openFor(pid)
            letterHandler: win.letterKey
            hiddenColumns: settings.hiddenColumns.split(",").filter(x => x.length > 0)
            onHiddenColumnsChanged: settings.hiddenColumns = hiddenColumns.join(",")
            shortenStorePaths: settings.shortenStorePaths
            onShortenStorePathsChanged: settings.shortenStorePaths = shortenStorePaths
        }

        // htop's function-key bar
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: keysRow.implicitHeight + 4
            color: win.dark ? "#16261c" : "#dcefe2"
            RowLayout {
                id: keysRow
                anchors.fill: parent
                anchors.leftMargin: 4
                anchors.rightMargin: 8
                spacing: 0
                Repeater {
                    model: [
                        { k: "F1", t: "Help", a: () => helpDialog.open() },
                        { k: "F3", t: "Search", a: () => filterField.forceActiveFocus() },
                        { k: "F5", t: win.view === "tree" ? "List" : "Tree", a: () => win.setView(win.view === "tree" ? "flat" : "tree") },
                        { k: "c", t: "Containers", a: () => win.setView(win.view === "containers" ? "tree" : "containers") },
                        { k: "g", t: "GPU", a: () => win.setView(win.view === "gpu" ? "tree" : "gpu") },
                        { k: "F7", t: "Nice-", a: () => table.renice(-1) },
                        { k: "F8", t: "Nice+", a: () => table.renice(1) },
                        { k: "F9", t: "Kill", a: () => table.openMenu(table.width / 3, table.height / 3) },
                        { k: "F10", t: "Hide", a: () => win.hide() },
                    ]
                    Row {
                        spacing: 0
                        Rectangle {
                            width: kt.implicitWidth + 6; height: kt.implicitHeight + 2
                            color: "transparent"
                            Text { id: kt; anchors.centerIn: parent; text: modelData.k; font.family: win.mono.family; font.pointSize: win.mono.pointSize; font.bold: true; color: win.textColor }
                        }
                        Rectangle {
                            width: tt.implicitWidth + 12; height: tt.implicitHeight + 2
                            color: "#2e9e5b"
                            Text { id: tt; anchors.centerIn: parent; text: modelData.t; font: win.mono; color: "#0d1a12" }
                            MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: modelData.a() }
                        }
                    }
                }
                Item { Layout.fillWidth: true }
                Text {
                    visible: table.frozen
                    text: "⏸ order held while the pointer is over the list  "
                    font: win.mono
                    color: win.dimColor
                }
                Text {
                    text: win.status
                    font: win.mono
                    color: win.cYellow
                    elide: Text.ElideLeft
                    Layout.maximumWidth: win.width * 0.45
                }
            }
        }
    }

    // ---- details dialog --------------------------------------------------
    Dialog {
        id: details
        property var info: ({})
        function openFor(pid) {
            info = JSON.parse(mon.details(pid))
            open()
        }
        title: "PID " + (info.pid || "") + " — " + (info.name || "")
        anchors.centerIn: parent
        width: Math.min(win.width - 80, 760)
        standardButtons: Dialog.Close
        modal: true
        GridLayout {
            anchors.fill: parent
            columns: 2
            columnSpacing: 12
            rowSpacing: 4
            Repeater {
                model: [
                    ["Command", details.info.command],
                    ["Executable", details.info.exe],
                    ["Working dir", details.info.cwd],
                    ["State", details.info.state],
                    ["Parent PID", details.info.ppid],
                    ["Threads", details.info.threads],
                    ["Open files", details.info.fds < 0 ? "(no permission)" : details.info.fds],
                    ["UIDs", details.info.uid],
                    ["Peak VM / swap", (details.info.vmPeak || "-") + " / " + (details.info.vmSwap || "-")],
                    ["Disk read / write", details.info.readBytes == null ? "(no permission)" : Fmt.bytes(details.info.readBytes) + " / " + Fmt.bytes(details.info.writeBytes)],
                    ["Context switches", (details.info.ctxtVoluntary || "-") + " voluntary, " + (details.info.ctxtInvoluntary || "-") + " forced"],
                    ["cgroup", details.info.cgroup],
                    ["Container", details.info.containerId ? (details.info.containerName || "?") + "  (" + details.info.engine + ")" : "—"],
                    ["Image", details.info.containerImage || "—"],
                    ["Container ID", details.info.containerId ? details.info.containerId.slice(0, 12) : "—"],
                    ["GPU", details.info.gpus ? "GPU" + details.info.gpus + " · VRAM " + Fmt.bytes(details.info.vram) + " · GTT " + Fmt.bytes(details.info.gtt)
                        + " · GFX/CMP/DMA/DEC/ENC " + (details.info.gpuEngines || []).map(v => v.toFixed(0) + "%").join(" / ") : "—"],
                ]
                delegate: Item {
                    Layout.columnSpan: 2
                    Layout.fillWidth: true
                    implicitHeight: Math.max(k.implicitHeight, v.implicitHeight)
                    Label { id: k; text: modelData[0]; color: win.dimColor; width: 140 }
                    TextEdit {
                        id: v
                        x: 150
                        width: parent.width - 150
                        text: modelData[1] === undefined || modelData[1] === null ? "" : String(modelData[1])
                        readOnly: true
                        selectByMouse: true
                        wrapMode: TextEdit.WrapAnywhere
                        color: win.textColor
                        font: win.mono
                    }
                }
            }
        }
    }

    Dialog {
        id: helpDialog
        title: "treetop keys"
        anchors.centerIn: parent
        standardButtons: Dialog.Close
        modal: true
        Label {
            font: win.mono
            text: "↑ ↓ PgUp PgDn Home End   move\n"
                + "Space         tag process (Ctrl+click too)\n"
                + "U             untag all\n"
                + "+ / − / ← →   expand / collapse subtree\n"
                + "*             expand all\n"
                + "Enter         details\n"
                + "F9 / k        signal menu (right-click too)\n"
                + "F7 / ]        nice − (higher priority)\n"
                + "F8 / [        nice + (lower priority)\n"
                + "F5 / t        tree ⇄ list\n"
                + "c             group by container\n"
                + "g             GPU view (amdgpu_top-style)\n"
                + "1             per-core ⇄ combined CPU meter\n"
                + "r             collapse / expand resources\n"
                + "K             show kernel threads\n"
                + "P / M / T     sort by CPU / memory / time\n"
                + "F3 / /        filter\n"
                + "Esc / F10 / q hide to tray     Ctrl+Q quit\n"
                + "Ctrl+= / Ctrl+- / Ctrl+0      font size\n"
                + "Right-click the header        choose columns\n\n"
                + "Signalling someone else's process (e.g. root's)\n"
                + "asks for authorization through polkit."
        }
    }
}
