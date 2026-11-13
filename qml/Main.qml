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
    // Shown by C++ after the saved size/position are applied (wantVisible).
    visible: false
    property bool wantVisible: !startHidden
    property bool restoreMaximized: false
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
    // VRAM gets its own colour (pink), shared with the panel widget.
    readonly property color cPink: dark ? "#d070a8" : "#a8407a"
    color: pal.window

    // ---- state -----------------------------------------------------------
    property var snap: ({ cpu: { cores: [] }, mem: {}, gpus: [], disk: { filesystems: [] }, net: {}, load: [0, 0, 0], procs: {} })
    readonly property int historyLength: 90
    property var hist: ({ cpu: [], mem: [], rx: [], tx: [], rd: [], wr: [], gpu: [], vram: [] })
    // GPUs the user hid (by PCI address), and the rest with their numbers.
    readonly property var hiddenGpus: settings.hiddenGpus.split(",").filter(x => x.length > 0)
    readonly property var pinnedGpus: settings.pinnedGpus.split(",").filter(x => x.length > 0)
    // A GPU that did nothing for the last minute (and doesn't drive a
    // display) folds away on its own; it comes back when it gets busy.
    function gpuIdle(i, g) {
        if (g.display) return false
        const h = hist.gpu[i] || []
        if (h.length < 20) return false
        const recent = h.slice(-60)
        const vram = g.vramTotal > 0 ? g.vramUsed / g.vramTotal * 100 : 0
        return Math.max(...recent) < 3 && vram < 10
    }
    readonly property var allGpus: snap.gpus.map((g, i) => Object.assign({ number: i + 1, index: i, idle: gpuIdle(i, g) }, g))
    readonly property var gpus: allGpus.filter(g => hiddenGpus.indexOf(g.pdev) < 0 && (!g.idle || pinnedGpus.indexOf(g.pdev) >= 0))
    readonly property var collapsed: settings.collapsedSections.split(",").filter(x => x.length > 0)
    function sectionOpen(key) { return collapsed.indexOf(key) < 0 }
    function toggleSection(key) {
        const c = collapsed.slice(), i = c.indexOf(key)
        if (i >= 0) c.splice(i, 1); else c.push(key)
        settings.collapsedSections = c.join(",")
    }
    // Packs the meter sections into columns: tries 1..5 columns and keeps
    // the arrangement with the fewest lines (ties: fewer columns).
    function updatePlan() {
        const w = meterArea.width
        if (w <= 0) return
        const nCores = snap.cpu.cores.length
        const fsCount = (snap.disk.filesystems || []).length
        const coreColsFor = (colW) => Math.max(1, Math.min(4, Math.floor(colW / 150)))
        const lines = (name, colW) => {
            switch (name) {
            case "cpu": return settings.cpuCombined ? 2 : 1 + Math.ceil(nCores / coreColsFor(colW))
            case "mem": return 1 + (sectionOpen("mem") ? ((snap.mem.swapTotal || 0) > 0 ? 2 : 1) : 0)
            case "gpu": return snap.gpus.length === 0 ? 0 : 1 + (sectionOpen("gpu") ? 3 * gpus.length : 0)
            case "storage": return 1 + (sectionOpen("storage") ? fsCount : 0)
            case "system": return 1 + (sectionOpen("system") ? 3 : 0)
            }
            return 1
        }
        const names = ["cpu", "mem", "gpu", "storage", "system"].filter(n => n !== "gpu" || snap.gpus.length > 0)
        let best = null
        for (let cols = 1; cols <= Math.min(5, names.length); cols++) {
            const colW = (w - meterArea.gap * (cols - 1)) / cols
            if (cols > 1 && colW < 300) break
            const heights = new Array(cols).fill(0)
            const columns = []
            for (let c = 0; c < cols; c++) columns.push([])
            for (const n of names) {
                let c = 0
                for (let i = 1; i < cols; i++) if (heights[i] < heights[c]) c = i
                columns[c].push(n)
                heights[c] += lines(n, colW) + 0.3
            }
            const h = Math.max(...heights)
            if (!best || h < best.h - 0.01) best = { h: h, cols: cols, colW: Math.floor(colW), coreCols: coreColsFor(colW), columns: columns.filter(c => c.length) }
        }
        if (!best) return
        // Columns that ended up empty give their width to the rest.
        if (best.columns.length < best.cols) {
            best.cols = best.columns.length
            best.colW = Math.floor((w - meterArea.gap * (best.cols - 1)) / best.cols)
            best.coreCols = coreColsFor(best.colW)
        }
        const plan = { cols: best.cols, colW: best.colW, coreCols: best.coreCols, columns: best.columns }
        if (JSON.stringify(plan) !== JSON.stringify(meterArea.plan)) meterArea.plan = plan
    }
    onGpusChanged: Qt.callLater(updatePlan)
    Connections {
        target: settings
        function onCpuCombinedChanged() { Qt.callLater(win.updatePlan) }
        function onCollapsedSectionsChanged() { Qt.callLater(win.updatePlan) }
    }
    readonly property var focusInfo: snap.procs && snap.procs.focus ? snap.procs.focus : null
    function setGpuShown(pdev, show) {
        if (show) {
            settings.hiddenGpus = hiddenGpus.filter(p => p !== pdev).join(",")
            // "Show" on an idle GPU keeps it shown even while idle.
            const g = allGpus.find(x => x.pdev === pdev)
            if (g && g.idle && pinnedGpus.indexOf(pdev) < 0) settings.pinnedGpus = pinnedGpus.concat([pdev]).join(",")
        } else hideGpu(pdev)
    }
    // GPUs not shown (hidden by you, or idle), each leaving a "+ show" behind.
    readonly property var hiddenGpuList: allGpus.filter(g => gpus.indexOf(g) < 0)
        .map(g => Object.assign({ reason: hiddenGpus.indexOf(g.pdev) >= 0 ? "hidden" : "idle" }, g))
    function hideGpu(pdev) {
        const h = hiddenGpus.slice(); if (h.indexOf(pdev) < 0) h.push(pdev)
        settings.hiddenGpus = h.join(",")
        settings.pinnedGpus = pinnedGpus.filter(p => p !== pdev).join(",")
    }
    property string status: ""
    property int collapsedCount: 0
    property string view: settings.view

    QtCore.Settings {
        id: settings
        property int interval: 1000
        property string view: "tree"
        property bool showMeters: true
        property bool showCards: true
        property bool showKernel: false
        property real fontDelta: 0
        property string hiddenColumns: "pri,ni,shr,gpus,engines,container,time"
        property int columnsVersion: 0
        property bool shortenStorePaths: true
        property string storeStyle: "compact"
        property bool showResources: true
        property bool cpuCombined: false
        property string hiddenGpus: ""
        property string pinnedGpus: ""
        // Restart detection: a --hidden start soon after an instance was
        // alive (devloop rebuild) restores the window instead of hiding it.
        property real lastAlive: 0
        property bool lastVisible: true
        property bool ownTrayIcon: false
        property string collapsedSections: ""
        // The view as you left it.
        property string sortKey: "cpu"
        property bool sortDesc: true
        property string filterText: ""
        property string selectedKey: ""
        property real scrollY: 0
        property real scrollX: 0
        property bool rootMode: false
        property string columnWidths: "{}"
    }

    Monitor {
        id: mon
        onUpdated: (json) => win.apply(JSON.parse(json))
    }
    property bool booted: false
    property bool autostart: false
    onVisibleChanged: {
        mon.set_rows_enabled(visible)
        if (booted) { settings.lastVisible = visible; settings.sync() }
    }
    Timer {
        interval: 5000; running: true; repeat: true
        onTriggered: { settings.lastAlive = Date.now(); settings.sync() }
    }
    function setAutostart(on) {
        showStatus(mon.set_autostart(on))
        autostart = mon.autostart_enabled()
    }
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
            const recount = !snap.cpu || snap.cpu.cores.length !== s.cpu.cores.length
                || (snap.disk.filesystems || []).length !== s.disk.filesystems.length
                || snap.gpus.length !== s.gpus.length
            snap = s
            if (s.top) top = s.top
            if (recount) Qt.callLater(updatePlan)
            for (const n of s.notices) showStatus(n)
        }
        if (s.collapsedCount !== undefined) collapsedCount = s.collapsedCount
        if (s.rows) table.applyDiff(s.rows)
    }

    function showStatus(text) {
        if (!text) return
        status = text
        statusTimer.restart()
    }
    Timer { id: statusTimer; interval: 6000; onTriggered: win.status = "" }
    // Scroll position and selection, saved shortly after they settle.
    Timer {
        id: viewSave
        interval: 500
        onTriggered: {
            settings.scrollY = table.scrollY
            settings.scrollX = table.scrollX
            settings.sync()
        }
    }

    Component.onCompleted: {
        // Restarted by a rebuild (an instance was alive moments ago): come
        // back the way it was. A real login start stays in the tray.
        if (startHidden && settings.lastVisible && Date.now() - settings.lastAlive < 30000) win.wantVisible = true
        booted = true
        // One-time: TIME+ is now hidden by default; apply that to saved settings too.
        if (settings.columnsVersion < 1) {
            const h = settings.hiddenColumns.split(",").filter(x => x.length > 0)
            if (h.indexOf("time") < 0) h.push("time")
            settings.hiddenColumns = h.join(",")
            settings.columnsVersion = 1
        }
        settings.lastAlive = Date.now()
        autostart = mon.autostart_enabled()
        mon.set_interval(settings.interval)
        mon.set_rows_enabled(win.visible)
        mon.set_view(settings.view)
        mon.set_show_kernel(settings.showKernel)
        table.sortKey = settings.sortKey
        table.sortDesc = settings.sortDesc
        mon.set_sort(table.sortKey, table.sortDesc)
        filterField.text = settings.filterText
        table.restoreKey = settings.selectedKey
        table.restoreY = settings.scrollY
        table.restoreX = settings.scrollX
        if (settings.rootMode) mon.set_root_mode(true)
    }

    function setView(v) {
        settings.view = v
        mon.set_view(v)
        if (v === "gpu" && table.sortKey !== "gpu") { table.sortKey = ""; table.sortBy("gpu") }
    }

    // ---- window/tray plumbing (called from C++ for single-instance) -----
    function showWindow() {
        if (win.restoreMaximized) win.showMaximized(); else win.show()
        win.raise()
        win.requestActivate()
    }
    function hideWindow() { win.hide() }
    // For the D-Bus Top() call (panel widget tooltips).
    property var top: ({})
    function topJson() { return JSON.stringify(top) }
    // Open (and not minimized) -> hide; otherwise show. Not "focused": a
    // click on the panel widget or tray takes focus away first.
    function toggleWindow() {
        if (win.visible && win.visibility !== Window.Minimized) win.hide(); else showWindow()
    }
    onClosing: (close) => {
        // Closing hides to the tray (our icon or the tray widget).
        if (tray.available || win.widgetInTray) { close.accepted = false; win.hide() }
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

    // The treetop widget in the Plasma tray replaces the app's own icon.
    property bool widgetInTray: false
    Timer {
        interval: 15000; running: true; repeat: true; triggeredOnStart: true
        onTriggered: win.widgetInTray = mon.tray_widget_present()
    }
    Platform.SystemTrayIcon {
        id: tray
        visible: available && (!win.widgetInTray || settings.ownTrayIcon)
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
            Platform.MenuItem { text: "Start at login"; checkable: true; checked: win.autostart; onTriggered: win.setAutostart(!win.autostart) }
            Platform.MenuSeparator {}
            Platform.MenuItem { text: "Quit"; onTriggered: Qt.quit() }
        }
    }

    // ---- shortcuts (htop keys) ------------------------------------------
    Shortcut { sequences: ["F3", "Ctrl+F"]; onActivated: filterField.forceActiveFocus() }
    Shortcut { sequence: "F5"; onActivated: win.setView(win.view === "tree" ? "flat" : "tree") }
    Shortcut { sequence: "F10"; onActivated: win.hide() }
    Shortcut { sequence: "Ctrl+Q"; onActivated: Qt.quit() }
    Shortcut {
        sequence: "Escape"
        onActivated: {
            if (filterField.text) { filterField.text = ""; table.forceActiveFocus() }
            else if (win.focusInfo) mon.set_focus(0)
            else win.hide()
        }
    }
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
                ToolSeparator { visible: win.view === "tree" || win.view === "containers" }
                ToolButton {
                    // One toggle: expands everything if anything is collapsed,
                    // otherwise collapses to the top-level processes.
                    visible: win.view === "tree" || win.view === "containers"
                    readonly property bool anyCollapsed: win.collapsedCount > 0
                    icon.name: anyCollapsed ? "expand-all" : "collapse-all"
                    text: anyCollapsed ? "Expand all" : "Collapse all"
                    onClicked: mon.set_expanded_all(anyCollapsed)
                    ToolTip.text: anyCollapsed ? "Expand every branch of the tree (*)" : "Collapse the tree to its top-level processes (_)"
                    ToolTip.visible: hovered
                }
                ToolSeparator {}
                TextField {
                    id: filterField
                    Layout.preferredWidth: 280
                    placeholderText: "Filter: name, command, user, container, PID  (/)"
                    onTextChanged: { mon.set_filter(text); settings.filterText = text }
                    Keys.onDownPressed: table.forceActiveFocus()
                    Keys.onReturnPressed: table.forceActiveFocus()
                }
                CheckBox { text: "Kernel threads"; checked: settings.showKernel; onToggled: { settings.showKernel = checked; mon.set_show_kernel(checked); table.refreshRows() } }
                Item { Layout.fillWidth: true }
                ToolButton {
                    // Root mode indicator / switch.
                    icon.name: win.snap.root ? "object-unlocked" : "object-locked"
                    text: win.snap.root ? "root" : ""
                    checkable: true
                    checked: !!win.snap.root
                    onClicked: { settings.rootMode = !win.snap.root; mon.set_root_mode(!win.snap.root) }
                    ToolTip.text: win.snap.root
                        ? "Root access is on: every process's disk I/O, GPU use, executable path, environment and open files are visible, and signals to root's processes need no further prompts. Click to turn off."
                        : "Root access is off: other users' (e.g. root's) disk I/O, GPU use, paths, environment and files are hidden by the kernel. Click to turn it on (asks polkit once)."
                    ToolTip.visible: hovered
                    ToolTip.delay: 400
                }
                Label { text: "Update"; color: win.dimColor }
                ComboBox {
                    model: [{ t: "0.25 s", v: 250 }, { t: "0.5 s", v: 500 }, { t: "1 s", v: 1000 }, { t: "2 s", v: 2000 }, { t: "5 s", v: 5000 }]
                    textRole: "t"
                    valueRole: "v"
                    Component.onCompleted: currentIndex = indexOfValue(settings.interval)
                    onActivated: settings.interval = currentValue
                }
                ToolButton {
                    icon.name: "application-menu"
                    ToolTip.text: "Options"
                    ToolTip.visible: hovered
                    onClicked: optionsMenu.popup(this, 0, height)
                    Menu {
                        id: optionsMenu
                        MenuItem { text: "Start at login (in the tray)"; checkable: true; checked: win.autostart; onTriggered: win.setAutostart(!win.autostart) }
                        MenuItem {
                            text: "Separate tray icon (besides the tray widget)"
                            visible: win.widgetInTray
                            height: visible ? implicitHeight : 0
                            checkable: true
                            checked: settings.ownTrayIcon
                            onTriggered: settings.ownTrayIcon = !settings.ownTrayIcon
                        }
                        MenuItem {
                            text: win.snap.rootStarting ? "Root access (waiting for polkit…)" : "Root access (polkit)"
                            checkable: true
                            checked: !!win.snap.root
                            enabled: !win.snap.rootStarting
                            onTriggered: { settings.rootMode = !win.snap.root; mon.set_root_mode(!win.snap.root) }
                        }
                        Menu {
                            title: "GPUs"
                            Repeater {
                                model: win.snap.gpus.length
                                MenuItem {
                                    readonly property var g: win.allGpus[index] || ({})
                                    text: "GPU" + (index + 1) + " — " + (g.name || "") + (g.idle ? "  (idle)" : "")
                                    checkable: true
                                    checked: win.gpus.some(x => x.pdev === g.pdev)
                                    onTriggered: win.setGpuShown(g.pdev, !win.gpus.some(x => x.pdev === g.pdev))
                                }
                            }
                        }
                        MenuSeparator {}
                        MenuItem { text: "Show resources (r)"; checkable: true; checked: settings.showResources; onTriggered: settings.showResources = !settings.showResources }
                        MenuItem { text: "Graphs"; checkable: true; checked: settings.showCards; onTriggered: settings.showCards = !settings.showCards }
                        MenuItem { text: "Meters"; checkable: true; checked: settings.showMeters; onTriggered: settings.showMeters = !settings.showMeters }
                        MenuItem { text: "Combine CPU cores (1)"; checkable: true; checked: settings.cpuCombined; onTriggered: settings.cpuCombined = !settings.cpuCombined }
                        MenuSeparator {}
                        MenuItem { text: "Keys… (h)"; icon.name: "help-contents"; onTriggered: helpDialog.open() }
                        MenuItem { text: "Quit"; icon.name: "application-exit"; onTriggered: Qt.quit() }
                    }
                }
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
                    // Count-based so the open menu isn't rebuilt by each update.
                    model: win.snap.gpus.length
                    MenuItem {
                        readonly property var g: win.snap.gpus[index] || ({})
                        text: "Show GPU" + (index + 1) + " — " + (g.name || "")
                        checkable: true
                        checked: win.hiddenGpus.indexOf(g.pdev) < 0
                        onTriggered: win.setGpuShown(g.pdev, !(win.hiddenGpus.indexOf(g.pdev) < 0))
                    }
                }
            }
        }

        // Which GPUs to show (from the GPU section's "+N hidden" link).
        Menu {
            id: gpuChooser
            MenuItem { text: "GPUs to show"; enabled: false }
            Repeater {
                model: win.snap.gpus.length
                MenuItem {
                    readonly property var g: win.allGpus[index] || ({})
                    text: "GPU" + (index + 1) + " — " + (g.name || "") + (g.idle ? "  (idle)" : "")
                    checkable: true
                    checked: win.gpus.some(x => x.pdev === g.pdev)
                    onTriggered: win.setGpuShown(g.pdev, !win.gpus.some(x => x.pdev === g.pdev))
                }
            }
        }

        // One context menu for every GPU card/meter, outside the Repeaters,
        // so updates can't destroy it while it's open.
        Menu {
            id: gpuMenu
            property string pdev: ""
            property int number: 0
            function openFor(g) { pdev = g.pdev; number = g.number; popup() }
            MenuItem { text: "Hide GPU " + gpuMenu.number; onTriggered: win.hideGpu(gpuMenu.pdev) }
            MenuItem { text: "GPU view (g)"; onTriggered: win.setView("gpu") }
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
                onClicked: settings.cpuCombined = !settings.cpuCombined
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
                model: win.gpus.length
                StatCard {
                    readonly property var g: win.gpus[index] || ({ busy: 0, number: 0, index: 0 })
                    Layout.fillWidth: true
                    title: "GPU " + g.number; accent: win.cMagenta; textColor: win.textColor; dimColor: win.dimColor; font: win.mono
                    collapsible: true
                    onCollapseRequested: win.hideGpu(g.pdev)
                    value: (g.busy || 0).toFixed(0) + "%"
                    detail: Fmt.bytes(g.vramUsed) + " / " + Fmt.bytes(g.vramTotal)
                        + (g.temp ? " · " + g.temp.toFixed(0) + "°C" : "")
                        + (g.power ? " · " + g.power.toFixed(0) + "W" : "")
                    series: [{ values: win.hist.gpu[g.index] || [], color: win.cMagenta }]
                    maximum: 100
                    ToolTip.text: (g.name || "") + "\nClick for the GPU view · right-click to hide"
                    ToolTip.visible: gpuHover.hovered
                    ToolTip.delay: 600
                    HoverHandler { id: gpuHover }
                    onClicked: win.setView(win.view === "gpu" ? "tree" : "gpu")
                    TapHandler { acceptedButtons: Qt.RightButton; onTapped: gpuMenu.openFor(parent.g) }
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
                model: win.gpus.length
                GpuPanel {
                    readonly property var g: win.gpus[index] || ({})
                    Layout.fillWidth: true
                    gpu: g
                    number: g.number || 0
                    history: win.hist.gpu[g.index] || []
                    vramHistory: win.hist.vram[g.index] || []
                    font: win.mono; textColor: win.textColor; dimColor: win.dimColor; trackColor: win.trackColor
                    accent: win.cMagenta; memColor: win.cCyan
                    onHideRequested: win.hideGpu(g.pdev)
                }
            }
            Repeater {
                model: win.hiddenGpuList.length
                HiddenChip {
                    readonly property var g: win.hiddenGpuList[index] || ({})
                    Layout.fillWidth: true
                    inline: true
                    label: "GPU" + g.number + " (" + (g.name || "") + ") is " + (g.reason === "idle" ? "idle" : "hidden")
                    onRestore: win.setGpuShown(g.pdev, true)
                }
            }
        }

        // htop half: meters, as collapsible sections packed into as many
        // columns as fit (each goes into the shortest column so far), so
        // folding one lets the others reflow instead of leaving a hole.
        Item {
            id: meterArea
            visible: settings.showResources && settings.showMeters && win.view !== "gpu"
            Layout.fillWidth: true
            Layout.leftMargin: 8
            Layout.rightMargin: 8
            Layout.topMargin: 6
            Layout.bottomMargin: 4
            implicitHeight: meterRow.implicitHeight
            readonly property real gap: 18
            // {cols, colW, coreCols, columns: [[section, ...], ...]}; replaced
            // only when the arrangement really changes, so sections aren't
            // rebuilt on every update.
            property var plan: ({ cols: 1, colW: 300, coreCols: 2, columns: [["cpu", "mem", "gpu", "storage", "system"]] })
            onWidthChanged: Qt.callLater(win.updatePlan)

            Row {
                id: meterRow
                spacing: meterArea.gap
                Repeater {
                    model: meterArea.plan.columns.length
                    Column {
                        readonly property var names: meterArea.plan.columns[index] || []
                        width: meterArea.plan.colW
                        spacing: 4
                        Repeater {
                            model: parent.names.length
                            Loader {
                                width: meterArea.plan.colW
                                sourceComponent: ({ cpu: cpuSection, mem: memSection, gpu: gpuSection, storage: storageSection, system: systemSection })[parent.names[index]]
                            }
                        }
                    }
                }
            }
        }

        Component {
            id: cpuSection
            ColumnLayout {
                spacing: 1
                SectionHeader {
                    Layout.fillWidth: true
                    title: "CPU"
                    open: !settings.cpuCombined
                    summary: Fmt.pct(win.snap.cpu.total || 0) + " · " + win.snap.cpu.cores.length + " threads"
                        + (win.snap.cpu.temp ? " · " + win.snap.cpu.temp.toFixed(0) + "°C" : "")
                    onToggled: settings.cpuCombined = !settings.cpuCombined
                }
                Meter {
                    visible: settings.cpuCombined
                    Layout.fillWidth: true
                    font: win.mono; textColor: win.textColor; trackColor: win.trackColor
                    label: "all"
                    labelWidth: 3.5 * win.mono.pointSize
                    segments: [
                        { value: win.snap.cpu.nice || 0, color: win.cBlue },
                        { value: win.snap.cpu.user || 0, color: win.cGreen },
                        { value: win.snap.cpu.system || 0, color: win.cRed },
                        { value: win.snap.cpu.other || 0, color: win.cOrange },
                    ]
                    text: "usr " + Fmt.pct(win.snap.cpu.user) + " sys " + Fmt.pct(win.snap.cpu.system) + "  " + Fmt.pct(win.snap.cpu.total || 0)
                }
                GridLayout {
                    id: coreGrid
                    visible: !settings.cpuCombined
                    Layout.fillWidth: true
                    readonly property int n: win.snap.cpu.cores.length
                    columns: meterArea.plan.coreCols
                    columnSpacing: 10
                    rowSpacing: 1
                    flow: GridLayout.TopToBottom
                    rows: Math.max(1, Math.ceil(n / columns))
                    Repeater {
                        // A count, not the array: meters stay alive and update.
                        model: settings.cpuCombined ? 0 : coreGrid.n
                        Meter {
                            readonly property var c: win.snap.cpu.cores[index] || [0, 0, 0, 0, 0]
                            Layout.fillWidth: true
                            font: win.mono; textColor: win.textColor; trackColor: win.trackColor
                            label: String(index)
                            labelWidth: 3 * win.mono.pointSize
                            segments: [
                                { value: c[1], color: win.cBlue },
                                { value: c[0], color: win.cGreen },
                                { value: c[2], color: win.cRed },
                                { value: c[3], color: win.cOrange },
                            ]
                            text: Fmt.pct(c[0] + c[1] + c[2] + c[3])
                        }
                    }
                }
            }
        }

        Component {
            id: memSection
            ColumnLayout {
                spacing: 1
                SectionHeader {
                    Layout.fillWidth: true
                    title: "Memory"
                    open: win.sectionOpen("mem")
                    summary: Fmt.bytes(win.snap.mem.used) + " / " + Fmt.bytes(win.snap.mem.total)
                        + ((win.snap.mem.swapTotal || 0) > 0 ? " · swap " + Fmt.bytes(win.snap.mem.swapUsed) : "")
                    onToggled: win.toggleSection("mem")
                }
                Meter {
                    visible: win.sectionOpen("mem")
                    Layout.fillWidth: true
                    font: win.mono; textColor: win.textColor; trackColor: win.trackColor
                    label: "Mem"
                    labelWidth: 5 * win.mono.pointSize
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
                    visible: win.sectionOpen("mem") && (win.snap.mem.swapTotal || 0) > 0
                    font: win.mono; textColor: win.textColor; trackColor: win.trackColor
                    label: "Swp"
                    labelWidth: 5 * win.mono.pointSize
                    maximum: win.snap.mem.swapTotal || 1
                    segments: [{ value: win.snap.mem.swapUsed, color: win.cRed }]
                    text: Fmt.bytes(win.snap.mem.swapUsed) + "/" + Fmt.bytes(win.snap.mem.swapTotal)
                }
            }
        }

        Component {
            id: gpuSection
            ColumnLayout {
                spacing: 1
                SectionHeader {
                    Layout.fillWidth: true
                    title: "GPU"
                    open: win.sectionOpen("gpu")
                    summary: win.gpus.map(g => "GPU" + g.number + " " + g.busy.toFixed(0) + "%").join(" · ")
                    // Hidden GPUs take no space; this link brings them back.
                    actionText: win.hiddenGpuList.length ? "+" + win.hiddenGpuList.length + " hidden" : ""
                    actionTip: win.hiddenGpuList.map(g => "GPU" + g.number + " (" + (g.name || "") + ") — " + g.reason).join("\n") + "\nClick to choose which GPUs to show"
                    onAction: gpuChooser.popup()
                    onToggled: win.toggleSection("gpu")
                }
                Repeater {
                    model: win.sectionOpen("gpu") ? win.gpus.length : 0
                    ColumnLayout {
                        readonly property var g: win.gpus[index] || ({ busy: 0, number: 0 })
                        Layout.fillWidth: true
                        spacing: 1
                        TapHandler { acceptedButtons: Qt.RightButton; onTapped: gpuMenu.openFor(parent.g) }
                        HoverHandler { id: gh }
                        ToolTip.text: (g.name || "") + "\n▾ hides it · the GPU card or g opens the GPU view"
                        ToolTip.visible: gh.hovered
                        ToolTip.delay: 800
                        Text {
                            // One click hides this GPU (it leaves "▸ GPUn hidden · + show").
                            text: "▾ GPU" + parent.g.number + "  " + (parent.g.name || "")
                            font: win.mono
                            color: hideHover.hovered ? win.cMagenta : win.dimColor
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                            HoverHandler { id: hideHover; cursorShape: Qt.PointingHandCursor }
                            TapHandler { onTapped: win.hideGpu(parent.parent.g.pdev) }
                        }
                        Meter {
                            Layout.fillWidth: true
                            font: win.mono; textColor: win.textColor; trackColor: win.trackColor
                            label: "busy"
                            labelWidth: 5 * win.mono.pointSize
                            segments: [{ value: parent.g.busy || 0, color: win.cMagenta }]
                            text: (parent.g.busy || 0).toFixed(0) + "%" + (parent.g.temp ? " " + parent.g.temp.toFixed(0) + "°C" : "") + (parent.g.power ? " " + parent.g.power.toFixed(0) + "W" : "")
                        }
                        Meter {
                            Layout.fillWidth: true
                            font: win.mono; textColor: win.textColor; trackColor: win.trackColor
                            label: "VRAM"
                            labelWidth: 5 * win.mono.pointSize
                            maximum: parent.g.vramTotal || 1
                            segments: [{ value: parent.g.vramUsed || 0, color: win.cPink }]
                            text: Fmt.bytes(parent.g.vramUsed) + "/" + Fmt.bytes(parent.g.vramTotal)
                        }
                    }
                }

            }
        }

        Component {
            id: storageSection
            ColumnLayout {
                spacing: 1
                SectionHeader {
                    Layout.fillWidth: true
                    title: "Storage"
                    open: win.sectionOpen("storage")
                    readonly property var fs: win.snap.disk.filesystems || []
                    summary: Fmt.bytes(fs.reduce((a, f) => a + f.used, 0)) + " / " + Fmt.bytes(fs.reduce((a, f) => a + f.total, 0))
                        + " · R " + Fmt.rate(win.snap.disk.read) + " W " + Fmt.rate(win.snap.disk.write)
                    onToggled: win.toggleSection("storage")
                }
                Repeater {
                    model: win.sectionOpen("storage") ? (win.snap.disk.filesystems || []).length : 0
                    Meter {
                        readonly property var f: win.snap.disk.filesystems[index] || ({ used: 0, total: 1, mount: "" })
                        Layout.fillWidth: true
                        font: win.mono; textColor: win.textColor; trackColor: win.trackColor
                        label: f.mount
                        labelWidth: 8 * win.mono.pointSize
                        maximum: f.total || 1
                        segments: [{ value: f.used, color: f.used / f.total > 0.9 ? win.cRed : win.cCyan }]
                        text: Fmt.bytes(f.used) + "/" + Fmt.bytes(f.total)
                        ToolTip.text: f.mount + "  ·  " + (f.device || "") + " (" + (f.type || "") + ")"
                        ToolTip.visible: fsHover.hovered
                        ToolTip.delay: 500
                        HoverHandler { id: fsHover }
                    }
                }
            }
        }

        Component {
            id: systemSection
            ColumnLayout {
                spacing: 1
                SectionHeader {
                    Layout.fillWidth: true
                    title: "System"
                    open: win.sectionOpen("system")
                    readonly property var p: win.snap.procs
                    summary: (p.tasks || 0) + " tasks · load " + win.snap.load.map(v => v.toFixed(2)).join(" ")
                    onToggled: win.toggleSection("system")
                }
                Text {
                    visible: win.sectionOpen("system")
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                    font: win.mono
                    color: win.textColor
                    textFormat: Text.StyledText
                    readonly property var p: win.snap.procs
                    text: "<font color='" + win.cCyan + "'>Tasks:</font> <b>" + (p.tasks || 0) + "</b>, " + (p.threads || 0) + " thr, "
                        + (p.kthreads || 0) + " kthr; <font color='" + win.cGreen + "'><b>" + (p.running || 0) + "</b> running</font>"
                        + (p.containers ? "; <font color='" + win.cMagenta + "'><b>" + p.containers + "</b> containers</font>" : "")
                }
                Text {
                    visible: win.sectionOpen("system")
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                    font: win.mono
                    color: win.textColor
                    textFormat: Text.StyledText
                    text: "<font color='" + win.cCyan + "'>Load:</font> <b>" + win.snap.load.map(v => v.toFixed(2)).join(" ") + "</b>"
                        + "  <font color='" + win.cCyan + "'>Up:</font> <b>" + Fmt.uptime(win.snap.uptime) + "</b>"
                }
                Text {
                    visible: win.sectionOpen("system")
                    Layout.fillWidth: true
                    elide: Text.ElideRight
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

        // Focus mode bar: where the focused process sits, and the way out.
        Rectangle {
            visible: !!win.focusInfo
            Layout.fillWidth: true
            implicitHeight: focusRow.implicitHeight + 8
            color: Qt.rgba(win.cCyan.r, win.cCyan.g, win.cCyan.b, 0.14)
            RowLayout {
                id: focusRow
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 8
                spacing: 6
                // Root of the breadcrumb: click to show everything again.
                Text {
                    text: "⌂ All"
                    font.family: win.mono.family
                    font.pointSize: win.mono.pointSize
                    font.bold: true
                    font.underline: allHover.hovered
                    color: allHover.hovered ? win.cCyan : win.textColor
                    HoverHandler { id: allHover; cursorShape: Qt.PointingHandCursor }
                    TapHandler { onTapped: mon.set_focus(0) }
                    ToolTip.text: "Show all processes again (Esc)"
                    ToolTip.visible: allHover.hovered
                    ToolTip.delay: 500
                }
                Text { text: "▸"; font: win.mono; color: win.dimColor }
                // Breadcrumb: click an ancestor to widen the focus to it.
                Repeater {
                    model: win.focusInfo ? win.focusInfo.chain.length : 0
                    Row {
                        spacing: 6
                        readonly property var a: win.focusInfo ? win.focusInfo.chain[index] : ({})
                        Text {
                            text: parent.a.name || ""
                            font.family: win.mono.family
                            font.pointSize: win.mono.pointSize
                            font.underline: crumbHover.hovered
                            color: crumbHover.hovered ? win.cCyan : win.dimColor
                            HoverHandler { id: crumbHover; cursorShape: Qt.PointingHandCursor }
                            TapHandler { onTapped: mon.set_focus(parent.parent.a.pid) }
                            ToolTip.text: "Focus on PID " + parent.a.pid + " instead"
                            ToolTip.visible: crumbHover.hovered
                            ToolTip.delay: 500
                        }
                        Text { text: "▸"; font: win.mono; color: win.dimColor }
                    }
                }
                Text {
                    text: win.focusInfo ? win.focusInfo.name + "  (PID " + win.focusInfo.pid + ", " + win.focusInfo.count + " processes)" : ""
                    font.family: win.mono.family
                    font.pointSize: win.mono.pointSize
                    font.bold: true
                    color: win.textColor
                }
                Item { Layout.fillWidth: true }
                ToolButton {
                    text: "✕ Show everything (Esc)"
                    font: win.mono
                    onClicked: mon.set_focus(0)
                }
            }
        }

        // The process table
        ProcessTable {
            id: table
            focusPid: win.focusInfo ? win.focusInfo.pid : 0
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
            widthOverride: { try { return JSON.parse(settings.columnWidths) } catch (e) { return ({}) } }
            onWidthOverrideChanged: settings.columnWidths = JSON.stringify(widthOverride)
            onSortKeyChanged: settings.sortKey = sortKey
            onSortDescChanged: settings.sortDesc = sortDesc
            onCurrentKeyChanged: if (restored) { settings.selectedKey = currentKey; viewSave.restart() }
            onScrollMoved: viewSave.restart()
            letterHandler: win.letterKey
            hiddenColumns: settings.hiddenColumns.split(",").filter(x => x.length > 0)
            onHiddenColumnsChanged: settings.hiddenColumns = hiddenColumns.join(",")
            shortenStorePaths: settings.shortenStorePaths
            onShortenStorePathsChanged: settings.shortenStorePaths = shortenStorePaths
            storeStyle: settings.storeStyle
            onStoreStyleChanged: settings.storeStyle = storeStyle
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
        property string envFilter: ""
        property bool asking: false
        function openFor(pid) {
            info = JSON.parse(mon.details(pid))
            envFilter = ""
            asking = false
            tabs.currentIndex = 0
            open()
        }
        function askRoot() {
            asking = true
            mon.inspect_as_root(info.pid)
        }
        Connections {
            target: mon
            function onInspected(json) {
                const r = JSON.parse(json)
                if (r.pid !== details.info.pid) return
                details.asking = false
                if (r.error) { win.showStatus(r.error); return }
                details.info = Object.assign({}, details.info, { environ: r.environ, files: r.files })
            }
        }
        title: "PID " + (info.pid || "") + " — " + (info.name || "")
        anchors.centerIn: parent
        width: Math.min(win.width - 80, 1000)
        height: Math.min(win.height - 80, 760)
        standardButtons: Dialog.Close
        modal: true

        // What a tab shows when the kernel hides the data.
        component NeedsRoot: ColumnLayout {
            property string what
            spacing: 10
            Label { text: "The kernel only shows a process's " + what + " to its owner or root."; color: win.dimColor }
            Button {
                text: details.asking ? "Waiting for authorization…" : "Read as root…"
                icon.name: "dialog-password"
                enabled: !details.asking
                onClicked: details.askRoot()
            }
            Label { text: "Or turn on Root access in ☰ Options to see everything without asking again."; color: win.dimColor; font.italic: true }
        }

        ColumnLayout {
            anchors.fill: parent
            spacing: 6
            TabBar {
                id: tabs
                Layout.fillWidth: true
                TabButton { text: "Overview" }
                TabButton { text: "Environment" + (details.info.environ ? " (" + details.info.environ.length + ")" : "") }
                TabButton { text: "Open files" + (details.info.files ? " (" + details.info.files.length + ")" : "") }
            }
            // Pages anchored to fill this area, shown by tab (StackLayout kept
            // a page hidden at first layout at a stale small size).
            Item {
                id: tabPages
                Layout.fillWidth: true
                Layout.fillHeight: true

                // ---- Overview
                ScrollView {
                    anchors.fill: parent
                    visible: tabs.currentIndex === 0
                    clip: true
                    GridLayout {
                        width: details.availableWidth - 20
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
                                ["Open files", details.info.files ? details.info.files.length : (details.info.fds < 0 ? "(needs root)" : details.info.fds)],
                                ["UIDs", details.info.uid],
                                ["Peak VM / swap", (details.info.vmPeak || "-") + " / " + (details.info.vmSwap || "-")],
                                ["Disk read / write", details.info.readBytes == null ? "(needs root)" : Fmt.bytes(details.info.readBytes) + " / " + Fmt.bytes(details.info.writeBytes)],
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
                                    readonly property bool isCommand: modelData[0] === "Command"
                                    x: 150
                                    width: parent.width - 150
                                    // The command reads as in its tooltip: full path, one
                                    // argument per line, coloured.
                                    textFormat: isCommand ? TextEdit.RichText : TextEdit.PlainText
                                    text: isCommand
                                        ? "<span style='font-family:" + win.mono.family + "'>" + table.prettyCommand(details.info.command || "", details.info.exe || "") + "</span>"
                                        : (modelData[1] === undefined || modelData[1] === null ? "" : String(modelData[1]))
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

                // ---- Environment
                ColumnLayout {
                    anchors.fill: parent
                    visible: tabs.currentIndex === 1
                    spacing: 6
                    NeedsRoot { visible: !details.info.environ; what: "environment" }
                    TextField {
                        visible: !!details.info.environ
                        Layout.fillWidth: true
                        placeholderText: "Filter variables"
                        text: details.envFilter
                        onTextChanged: details.envFilter = text
                    }
                    ListView {
                        visible: !!details.info.environ
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        id: envList
                        KineticScroll { view: envList; step: 72 }
                        KineticScroll { view: envList; orientation: Qt.Horizontal }
                        model: (details.info.environ || []).filter(kv => !details.envFilter
                            || (kv[0] + "=" + kv[1]).toLowerCase().indexOf(details.envFilter.toLowerCase()) >= 0)
                        ScrollBar.vertical: ScrollBar {}
                        delegate: TextEdit {
                            required property var modelData
                            width: ListView.view ? ListView.view.width - 14 : 400
                            readOnly: true
                            selectByMouse: true
                            wrapMode: TextEdit.WrapAnywhere
                            textFormat: TextEdit.RichText
                            font: win.mono
                            color: win.textColor
                            // NAME=value, with PATH-like lists one entry per line.
                            text: {
                                const k = modelData[0], v = modelData[1]
                                const list = v.indexOf(":") > 0 && v.split(":").length > 2 && v.indexOf("://") < 0
                                const val = list
                                    ? v.split(":").map(p => table.valueHtml(p)).join("<br>" + "&nbsp;".repeat(k.length + 1))
                                    : table.valueHtml(v)
                                return "<span style='font-family:" + win.mono.family + "'>" + table.span(table.syn.flag, k) + table.span(table.syn.eq, "=") + val + "</span>"
                            }
                        }
                    }
                }

                // ---- Open files
                ColumnLayout {
                    anchors.fill: parent
                    visible: tabs.currentIndex === 2
                    spacing: 6
                    NeedsRoot { visible: !details.info.files; what: "open files" }
                    ListView {
                        visible: !!details.info.files
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        id: filesList
                        KineticScroll { view: filesList; step: 72 }
                        KineticScroll { view: filesList; orientation: Qt.Horizontal }
                        model: details.info.files || []
                        ScrollBar.vertical: ScrollBar {}
                        delegate: Row {
                            required property var modelData
                            spacing: 10
                            Text { width: 5 * win.mono.pointSize; horizontalAlignment: Text.AlignRight; text: modelData.fd; font: win.mono; color: win.dimColor }
                            Text {
                                width: 8 * win.mono.pointSize
                                text: modelData.kind
                                font: win.mono
                                color: ({ socket: win.cBlue, pipe: win.cYellow, device: win.cMagenta, anon: win.dimColor, memfd: win.dimColor })[modelData.kind] || win.cGreen
                            }
                            TextEdit {
                                text: modelData.target
                                readOnly: true
                                selectByMouse: true
                                font: win.mono
                                color: win.textColor
                            }
                        }
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
                + "Shift+← →     scroll sideways (Shift+wheel too)\n"
                + "* / _         expand all / collapse all\n"
                + "Ctrl+→ / Ctrl+←  expand / collapse everything below\n"
                + "Click ▸/▾ (or the tree lines); Shift+click: whole subtree\n"
                + "Enter         details\n"
                + "f / double-click  show only this process tree (again or Esc: all)\n"
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
