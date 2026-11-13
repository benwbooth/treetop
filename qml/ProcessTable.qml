import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Fmt.js" as Fmt

// The htop half: a sortable process tree with tree lines, tagging, a signal
// menu and htop's keys. Rust sends row diffs; applyDiff() patches the
// ListModel in place and keeps the selected process at the same spot on
// screen, so the list doesn't jump around.
FocusScope {
    id: table
    property var monitor
    property font font
    property color textColor
    property color dimColor
    property color baseColor
    property color altColor
    property color highlightColor
    property color highlightTextColor
    property string sortKey: "cpu"
    property bool sortDesc: true
    property string view: "tree"
    property string currentKey: ""
    property int currentPid: 0
    property var tagged: ({})
    property var letterHandler: null
    property var hiddenColumns: []
    property bool shortenStorePaths: true
    readonly property bool frozen: listHover.hovered
    signal notice(string text)
    signal showDetails(int pid)

    readonly property real cw: fm.advanceWidth("0")
    readonly property real rowHeight: Math.ceil(fm.height) + 3
    FontMetrics { id: fm; font: table.font }

    readonly property var columns: [
        { key: "pid", title: "PID", chars: 8, right: true,
          tip: "Process ID. The kernel's number for this process; signals and most tools refer to processes by it." },
        { key: "user", title: "USER", chars: 10,
          tip: "The account the process runs as (its real user). Inside containers this is the UID as seen from the host, so it may show as a bare number." },
        { key: "pri", title: "PRI", chars: 4, right: true,
          tip: "Kernel scheduling priority. For normal processes this is 20 + nice; lower means scheduled sooner. \"RT\" marks real-time processes, which always run before normal ones." },
        { key: "ni", title: "NI", chars: 4, right: true,
          tip: "Nice value, from -20 (greedy, highest priority) to 19 (polite, lowest). Changed with F7/F8; lowering it below the current value needs authorization." },
        { key: "virt", title: "VIRT", chars: 7, right: true,
          tip: "Virtual memory size: everything the process has mapped — code, libraries, heap, memory-mapped files and reserved-but-unused address space. Usually far bigger than what it really uses; RES is the better measure." },
        { key: "res", title: "RES", chars: 7, right: true,
          tip: "Resident memory: the part of the process that is actually in RAM right now, including shared libraries it uses. This is the number to look at for \"how much memory is it using\"." },
        { key: "shr", title: "SHR", chars: 7, right: true,
          tip: "Shared memory: the portion of RES backed by files or shared with other processes (libraries, shared buffers). Pages counted here may also be counted in other processes' RES." },
        { key: "state", title: "S", chars: 2, right: true,
          tip: "State. R running or runnable, S sleeping (waiting for something), D uninterruptible sleep (usually disk or driver I/O; can't be killed until it returns), T stopped, Z zombie (exited, waiting for its parent to collect it), I idle kernel thread." },
        { key: "cpu", title: "CPU%", chars: 6, right: true,
          tip: "Share of CPU time over the last update, where 100% is one full core. A process using several cores can go above 100%, up to 100% × the number of cores." },
        { key: "mem", title: "MEM%", chars: 6, right: true,
          tip: "Resident memory (RES) as a percentage of total physical RAM." },
        { key: "gpu", title: "GPU%", chars: 6, right: true,
          tip: "How busy this process keeps the GPU: the busiest of its GPU engines (graphics, compute, copy, video decode/encode) over the last update, from the kernel's DRM fdinfo — the same source amdgpu_top and nvtop use. Only processes you own are visible without root." },
        { key: "vram", title: "GMEM", chars: 7, right: true,
          tip: "Video memory (VRAM) this process has allocated on the GPU, as reported by the driver. Buffers it merely imported from other processes are not counted." },
        { key: "gpus", title: "GPU#", chars: 5,
          tip: "Which GPU(s) the process has open, numbered as in the GPU cards above (GPU1, GPU2, ...)." },
        { key: "engines", title: "GFX/CMP/DMA/DEC/ENC", chars: 20,
          tip: "Per-engine GPU usage in %: GFX (3D/graphics), CMP (compute: ML, OpenCL, ROCm), DMA (copy engine), DEC (video decode), ENC (video encode)." },
        { key: "time", title: "TIME+", chars: 10, right: true,
          tip: "Total CPU time the process has used since it started (user + system), in minutes:seconds.hundredths, or hours once it passes an hour. A busy process accumulates time quickly; a long-lived idle one barely moves." },
        { key: "container", title: "CONTAINER", chars: 16, optional: true,
          tip: "The Docker/Podman/containerd container the process belongs to, worked out from its cgroup. Container processes also get a coloured stripe, and the first process of each container a badge, in the Command column." },
        { key: "command", title: "Command", chars: 0,
          tip: "The full command line. In tree view the lines show which process started which; ▸/▾ collapse and expand children (or +/−). The program name is in bold." },
    ]
    property bool showContainerColumn: true
    readonly property var forcedColumns: view === "gpu" ? ["gpu", "vram", "gpus", "engines"] : []
    readonly property var visibleColumns: columns.filter(c =>
        c.key === "command" || forcedColumns.indexOf(c.key) >= 0
        || ((!c.optional || showContainerColumn) && hiddenColumns.indexOf(c.key) < 0))

    function toggleColumn(key) {
        const h = hiddenColumns.slice()
        const i = h.indexOf(key)
        if (i >= 0) h.splice(i, 1); else h.push(key)
        hiddenColumns = h
    }

    // /nix/store/<32-char hash>-name -> /nix/store/…-name
    readonly property var storeHash: /\/nix\/store\/[0-9a-df-np-sv-z]{32}-/g
    function tidy(s) {
        return shortenStorePaths ? s.replace(storeHash, "/nix/store/…-") : s
    }

    function colWidth(c) {
        return c.chars > 0 ? (c.chars + 1) * cw : Math.max(20 * cw, list.contentWidth - fixedWidth)
    }
    readonly property real fixedWidth: fixedChars * cw
    readonly property int fixedChars: {
        let n = 0
        for (const c of visibleColumns) if (c.chars > 0) n += c.chars + 1
        return n
    }
    readonly property var fixedColumns: visibleColumns.filter(c => c.chars > 0)

    // One row's fixed columns as a single monospace run (one Text item per
    // row instead of one per cell keeps scrolling cheap).
    function pad(t, c) {
        t = String(t)
        if (t.length > c.chars) t = t.slice(0, c.chars - 1) + "…"
        return (c.right ? t.padStart(c.chars) : t.padEnd(c.chars)) + " "
    }

    ListModel { id: rows }
    property var keys: []   // mirror of rows' keys, for fast lookup
    // Longest command line seen (in characters): how far rows scroll sideways.
    property int maxLineChars: 0
    onViewChanged: maxLineChars = 0
    onShortenStorePathsChanged: maxLineChars = 0

    function applyDiff(d) {
        // Where is the selected row on screen right now?
        const oldIdx = keys.indexOf(currentKey)
        const offset = oldIdx >= 0 ? oldIdx * rowHeight - list.contentY : -1
        const wasVisible = offset >= 0 && offset < list.height

        const k = keys
        let longest = maxLineChars
        for (const [i, row] of d.changed) {
            if (i < rows.count) rows.set(i, row); else rows.append(row)
            k[i] = row.key
            const n = row.prefix.length + 2 + tidy(row.command).length + (row.container ? row.container.length + 6 : 0)
            if (n > longest) longest = n
        }
        if (longest !== maxLineChars) maxLineChars = longest
        if (rows.count > d.count) rows.remove(d.count, rows.count - d.count)
        k.length = d.count

        let idx = k.indexOf(currentKey)
        if (idx < 0 && d.count > 0) {
            // The selected process exited: stay on the same line.
            idx = Math.min(Math.max(0, oldIdx), d.count - 1)
            currentKey = k[idx]
            currentPid = rows.get(idx).pid
        }
        if (idx >= 0 && list.currentIndex !== idx) list.currentIndex = idx
        // Keep it at the same height on screen (rows above may have come or gone).
        if (wasVisible && idx >= 0) {
            const maxY = Math.max(0, rows.count * rowHeight - list.height)
            list.contentY = Math.max(0, Math.min(maxY, idx * rowHeight - offset))
        }
    }

    function selectedPids() {
        const t = Object.keys(tagged).map(Number).filter(p => p > 0)
        return t.length > 0 ? t : (currentPid > 0 ? [currentPid] : [])
    }

    function signalSelected(sig) {
        const pids = selectedPids()
        if (pids.length === 0) return
        notice(monitor.send_signal(pids.join(","), sig))
        tagged = ({})
    }

    function renice(delta) {
        if (currentPid <= 0 || list.currentIndex < 0) return
        const r = rows.get(list.currentIndex)
        const nice = Math.max(-20, Math.min(19, r.ni + delta))
        notice(monitor.renice(selectedPids().join(","), nice))
    }

    function toggleCurrent() {
        if (list.currentIndex < 0) return
        const r = rows.get(list.currentIndex)
        if (r.hasChildren) monitor.toggle(r.key)
    }

    function setCollapsed(collapsed) {
        if (list.currentIndex < 0) return
        const r = rows.get(list.currentIndex)
        if (r.hasChildren && r.expanded === collapsed) monitor.toggle(r.key)
    }

    function refreshRows() { monitor.refresh() }

    function sortBy(key) {
        if (key === sortKey) sortDesc = !sortDesc
        else { sortKey = key; sortDesc = !(key === "user" || key === "command" || key === "pid" || key === "container" || key === "gpus") }
        monitor.set_sort(sortKey, sortDesc)
    }

    function openMenu(x, y) {
        menu.popup(x, y)
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // Column header: click to sort (again to reverse), right-click to
        // choose columns, hover for an explanation.
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: table.rowHeight + 3
            color: "#2e9e5b"
            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.RightButton
                onClicked: (mouse) => { const p = mapToItem(table, mouse.x, mouse.y); columnMenu.popup(p.x, p.y) }
            }
            clip: true
            // Always-visible way to the column chooser (also: right-click).
            Rectangle {
                z: 2
                anchors.right: parent.right
                height: parent.height
                width: colBtn.implicitWidth + 12
                color: colHover.hovered ? "#57c785" : "#2e9e5b"
                Text {
                    id: colBtn
                    anchors.centerIn: parent
                    text: table.hiddenColumns.length > 0 ? "⋯ columns (" + table.hiddenColumns.length + " hidden)" : "⋯ columns"
                    font.family: table.font.family
                    font.pointSize: table.font.pointSize
                    font.bold: true
                    color: "#0d1a12"
                }
                HoverHandler { id: colHover; cursorShape: Qt.PointingHandCursor }
                TapHandler { onTapped: { const p = parent.mapToItem(table, 0, parent.height); columnMenu.popup(p.x, p.y) } }
                ToolTip.visible: colHover.hovered
                ToolTip.delay: 500
                ToolTip.text: "Choose which columns to show (you can also right-click any column header)."
            }
            Row {
                x: -list.contentX
                anchors.verticalCenter: parent.verticalCenter
                Repeater {
                    model: table.visibleColumns
                    Rectangle {
                        width: table.colWidth(modelData)
                        height: table.rowHeight + 3
                        color: modelData.key === table.sortKey ? "#57c785" : (hh.hovered ? "#3bb06b" : "transparent")
                        Text {
                            anchors.fill: parent
                            anchors.leftMargin: 4
                            anchors.rightMargin: table.cw - 4
                            verticalAlignment: Text.AlignVCenter
                            horizontalAlignment: modelData.right ? Text.AlignRight : Text.AlignLeft
                            text: modelData.title + (modelData.key === table.sortKey ? (table.sortDesc ? "▾" : "▴") : "")
                            font.family: table.font.family
                            font.pointSize: table.font.pointSize
                            font.bold: true
                            color: "#0d1a12"
                            elide: Text.ElideRight
                        }
                        HoverHandler { id: hh }
                        ToolTip.visible: hh.hovered
                        ToolTip.delay: 600
                        ToolTip.timeout: 20000
                        ToolTip.text: "<b>" + modelData.title + "</b><br>" + modelData.tip
                            + "<br><br><i>Click to sort, right-click to choose columns.</i>"
                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            acceptedButtons: Qt.LeftButton | Qt.RightButton
                            onClicked: (mouse) => {
                                if (mouse.button === Qt.RightButton) {
                                    const p = mapToItem(table, mouse.x, mouse.y)
                                    columnMenu.popup(p.x, p.y)
                                } else {
                                    table.sortBy(modelData.key === "command" ? "pid" : (modelData.key === "engines" ? "gpu" : modelData.key))
                                }
                            }
                        }
                    }
                }
            }
        }

        ListView {
            id: list
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: rows
            focus: true
            reuseItems: true
            cacheBuffer: 400
            boundsBehavior: Flickable.StopAtBounds
            highlightMoveDuration: 0
            keyNavigationEnabled: true
            ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }
            ScrollBar.horizontal: ScrollBar { policy: ScrollBar.AsNeeded }
            // Rows are as wide as the longest command line, and scroll sideways.
            contentWidth: Math.max(width, table.fixedWidth + 8 + (table.maxLineChars + 4) * table.cw)
            flickableDirection: Flickable.AutoFlickIfNeeded
            HoverHandler {
                id: listHover
                onHoveredChanged: table.monitor.set_frozen(hovered)
            }
            flickDeceleration: 2500
            maximumFlickVelocity: 12000

            // Smooth scrolling. Mouse wheels glide to an accumulated target
            // (faster when spun quickly); touchpads track the finger and
            // keep coasting with momentum when it lifts.
            property real wheelTarget: 0
            property real lastWheel: 0
            property real tpVelocity: 0
            property real tpLast: 0
            NumberAnimation {
                id: wheelAnim
                target: list
                property: "contentY"
                duration: 180
                easing.type: Easing.OutCubic
            }
            function clampY(y) { return Math.max(0, Math.min(y, Math.max(0, contentHeight - height))) }
            function clampX(x) { return Math.max(0, Math.min(x, Math.max(0, contentWidth - width))) }
            function scrollX(dx) { contentX = clampX(contentX + dx) }
            WheelHandler {
                target: null
                acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
                onWheel: (ev) => {
                    const now = Date.now()
                    // Sideways: touchpad swipes, tilt wheels, or Shift+wheel.
                    if (ev.pixelDelta.x !== 0) list.scrollX(-ev.pixelDelta.x)
                    else if (ev.angleDelta.x !== 0) list.scrollX(-ev.angleDelta.x / 120 * table.cw * 8)
                    if ((ev.modifiers & Qt.ShiftModifier) && ev.angleDelta.y !== 0 && ev.pixelDelta.y === 0) {
                        list.scrollX(-ev.angleDelta.y / 120 * table.cw * 8)
                        ev.accepted = true
                        return
                    }
                    if (ev.pixelDelta.y !== 0) {
                        // Touchpad: follow directly, remember speed for the fling.
                        wheelAnim.stop()
                        list.cancelFlick()
                        const dy = -ev.pixelDelta.y
                        const dt = Math.max(1, now - list.tpLast)
                        list.tpVelocity = list.tpVelocity * 0.6 + (dy / dt * 1000) * 0.4
                        list.tpLast = now
                        list.contentY = list.clampY(list.contentY + dy)
                        flingTimer.restart()
                    } else if (ev.angleDelta.y !== 0) {
                        const fast = now - list.lastWheel < 60 ? 2.2 : (now - list.lastWheel < 140 ? 1.5 : 1)
                        if (!wheelAnim.running) list.wheelTarget = list.contentY
                        list.lastWheel = now
                        list.wheelTarget = list.clampY(list.wheelTarget - ev.angleDelta.y / 120 * table.rowHeight * 3 * fast)
                        wheelAnim.stop()
                        wheelAnim.from = list.contentY
                        wheelAnim.to = list.wheelTarget
                        wheelAnim.start()
                    }
                    ev.accepted = true
                }
            }
            Timer {
                // No touchpad events for a moment: the fingers lifted, so coast.
                id: flingTimer
                interval: 40
                onTriggered: {
                    if (Math.abs(list.tpVelocity) > 150) list.flick(0, -list.tpVelocity)
                    list.tpVelocity = 0
                }
            }
            onCurrentIndexChanged: {
                if (currentIndex >= 0 && currentIndex < rows.count) {
                    const r = rows.get(currentIndex)
                    table.currentKey = r.key
                    table.currentPid = r.pid
                }
            }

            delegate: Rectangle {
                id: rowItem
                required property int index
                required property string key
                required property int pid
                required property int depth
                required property string prefix
                required property bool hasChildren
                required property bool expanded
                required property bool group
                required property string user
                required property int pri
                required property int ni
                required property real virt
                required property real res
                required property real shr
                required property string state
                required property real cpu
                required property real mem
                required property real time
                required property string container
                required property string engine
                required property string name
                required property string command
                required property bool kernel
                required property int count
                required property bool containerRoot
                required property real gpu
                required property real vram
                required property string gpus
                required property string gpuEngines

                readonly property bool current: ListView.isCurrentItem
                readonly property bool isTagged: !!table.tagged[pid]
                readonly property color fg: current ? table.highlightTextColor : (kernel ? table.dimColor : table.textColor)
                readonly property color chip: container.length > 0 && container !== "Host" ? Qt.hsla(Fmt.hue(container), 0.6, 0.55, 1) : "transparent"
                readonly property bool showBadge: chip.a > 0
                    && (group || (table.view !== "containers" && (containerRoot || table.view === "flat" || table.view === "gpu")))

                width: list.contentWidth
                height: table.rowHeight
                color: current ? table.highlightColor
                     : (group ? Qt.rgba(chip.r, chip.g, chip.b, 0.16)
                     : (container.length > 0 ? Qt.tint(index % 2 ? table.altColor : table.baseColor, Qt.rgba(chip.r, chip.g, chip.b, 0.06))
                     : (index % 2 ? table.altColor : table.baseColor)))

                function cell(k) {
                    switch (k) {
                    case "pid": return group ? "" : String(pid)
                    case "user": return user
                    case "pri": return group ? "" : (pri <= -100 ? "RT" : String(pri))
                    case "ni": return group ? "" : String(ni)
                    case "virt": return group ? "" : Fmt.bytes(virt)
                    case "res": return Fmt.bytes(res)
                    case "shr": return group ? "" : Fmt.bytes(shr)
                    case "state": return state
                    case "cpu": return cpu.toFixed(1)
                    case "mem": return mem.toFixed(1)
                    case "gpu": return gpus.length > 0 || gpu > 0 ? gpu.toFixed(1) : ""
                    case "vram": return vram > 0 ? Fmt.bytes(vram) : ""
                    case "gpus": return gpus
                    case "engines": return gpuEngines
                    case "time": return group ? count + (count === 1 ? " proc" : " procs") : Fmt.cpuTime(time)
                    case "container": return group ? "" : container
                    }
                    return ""
                }
                function cellColor(k) {
                    if (current) return table.highlightTextColor
                    if (k === "state" && state === "R") return "#43c96b"
                    if (k === "state" && state === "D") return "#e05a4f"
                    if (k === "state" && (state === "Z" || state === "T")) return "#d6a72e"
                    if (k === "cpu" && cpu >= 50) return "#e05a4f"
                    if (k === "gpu" && gpu >= 50) return "#c06fd8"
                    if (k === "res" && res >= 1073741824) return "#4fb3d9"
                    if (k === "vram" && vram >= 1073741824) return "#c06fd8"
                    if (k === "user" && user === "root") return "#e0794f"
                    if (k === "container") return chip
                    return fg
                }
                function commandHtml() {
                    // htop highlights the program's basename within the command line.
                    const cmd = table.tidy(command)
                    const sp = cmd.indexOf(" ")
                    const prog = sp < 0 ? cmd : cmd.slice(0, sp)
                    const base = prog.slice(prog.lastIndexOf("/") + 1)
                    const head = prog.slice(0, prog.length - base.length)
                    const rest = sp < 0 ? "" : cmd.slice(sp)
                    const dim = current ? table.highlightTextColor : table.dimColor
                    return "<span style='color:" + dim + "'>" + Fmt.escape(head) + "</span><b>" + Fmt.escape(base) + "</b>" + Fmt.escape(rest)
                }

                function fixedHtml() {
                    let out = ""
                    for (const c of table.fixedColumns) {
                        const txt = table.pad(cell(c.key), c)
                        const col = cellColor(c.key)
                        out += col === fg ? Fmt.escape(txt) : "<font color='" + col + "'>" + Fmt.escape(txt) + "</font>"
                    }
                    return out.replace(/ /g, "&nbsp;")
                }

                Text {
                    x: 4
                    width: table.fixedWidth
                    height: parent.height
                    verticalAlignment: Text.AlignVCenter
                    textFormat: Text.StyledText
                    text: rowItem.fixedHtml()
                    font: table.font
                    color: rowItem.fg
                    clip: true
                }

                Row {
                    x: table.fixedWidth + 4
                    width: parent.width - x
                    height: parent.height
                    clip: true
                    spacing: 0
                    // Container stripe: every process in a container.
                    Rectangle {
                        width: 3; height: rowItem.height
                        color: rowItem.group ? "transparent" : rowItem.chip
                    }
                    Item { width: 3; height: 1 }
                    Text {
                        // htop's tree lines; container groups indent one level first.
                        text: (rowItem.depth > 0 && table.view === "containers" && rowItem.prefix === "" ? "  " : "") + rowItem.prefix
                        font: table.font
                        color: rowItem.current ? table.highlightTextColor : table.dimColor
                        anchors.verticalCenter: parent.verticalCenter
                    }
                    Text {
                        width: table.cw * 2
                        text: rowItem.hasChildren && table.view !== "flat" && table.view !== "gpu" ? (rowItem.expanded ? "▾" : "▸") : ""
                        font: table.font
                        color: rowItem.fg
                        anchors.verticalCenter: parent.verticalCenter
                        MouseArea {
                            anchors.fill: parent
                            anchors.margins: -2
                            enabled: rowItem.hasChildren
                            onClicked: table.monitor.toggle(rowItem.key)
                        }
                    }
                    // Container badge: on a container group row, on the first
                    // process inside each container in the tree, and on every
                    // row in flat views.
                    Loader {
                        active: rowItem.showBadge
                        visible: active
                        anchors.verticalCenter: parent.verticalCenter
                        sourceComponent: Rectangle {
                            width: badge.implicitWidth + 10
                            height: rowItem.height - 3
                            radius: height / 2
                            color: Qt.rgba(rowItem.chip.r, rowItem.chip.g, rowItem.chip.b, 0.25)
                            border.color: rowItem.chip
                            Text {
                                id: badge
                                anchors.centerIn: parent
                                text: "⬢ " + rowItem.container + (rowItem.engine && rowItem.engine !== "docker" ? " · " + rowItem.engine : "")
                                font.family: table.font.family
                                font.pointSize: table.font.pointSize - 1
                                font.bold: true
                                color: rowItem.current ? table.highlightTextColor : rowItem.chip
                            }
                        }
                    }
                    Item { width: rowItem.showBadge ? 6 : 0; height: 1 }
                    Text {
                        text: rowItem.group ? Fmt.escape(table.tidy(rowItem.command)) : rowItem.commandHtml()
                        textFormat: Text.StyledText
                        font: table.font
                        color: rowItem.group ? (rowItem.current ? table.highlightTextColor : table.dimColor) : rowItem.fg
                        anchors.verticalCenter: parent.verticalCenter
                    }
                }

                Rectangle {
                    visible: rowItem.isTagged
                    width: 3; height: parent.height
                    color: "#e0c14f"
                }

                MouseArea {
                    anchors.fill: parent
                    z: -1
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    onPressed: (mouse) => {
                        list.currentIndex = rowItem.index
                        list.forceActiveFocus()
                        if (mouse.button === Qt.LeftButton && (mouse.modifiers & Qt.ControlModifier) && rowItem.pid > 0) {
                            const t = Object.assign({}, table.tagged)
                            if (t[rowItem.pid]) delete t[rowItem.pid]; else t[rowItem.pid] = true
                            table.tagged = t
                        }
                    }
                    onClicked: (mouse) => {
                        if (mouse.button === Qt.RightButton) {
                            const p = mapToItem(table, mouse.x, mouse.y)
                            table.openMenu(p.x, p.y)
                        }
                    }
                    onDoubleClicked: (mouse) => {
                        if (rowItem.hasChildren && rowItem.group) table.monitor.toggle(rowItem.key)
                        else if (rowItem.pid > 0) table.showDetails(rowItem.pid)
                    }
                }
            }

            Keys.onPressed: (event) => {
                const t = event.text
                if (event.key === Qt.Key_Space) {
                    if (table.currentPid > 0) {
                        const tg = Object.assign({}, table.tagged)
                        if (tg[table.currentPid]) delete tg[table.currentPid]; else tg[table.currentPid] = true
                        table.tagged = tg
                    }
                    list.incrementCurrentIndex()
                } else if ((event.modifiers & Qt.ShiftModifier) && event.key === Qt.Key_Right) {
                    list.scrollX(table.cw * 8)
                } else if ((event.modifiers & Qt.ShiftModifier) && event.key === Qt.Key_Left) {
                    list.scrollX(-table.cw * 8)
                } else if (t === "+" || t === "=" || event.key === Qt.Key_Right) {
                    table.setCollapsed(false)
                } else if (t === "-" || event.key === Qt.Key_Left) {
                    table.setCollapsed(true)
                } else if (t === "*") {
                    table.monitor.set_expanded_all(true)
                } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                    if (table.currentPid > 0) table.showDetails(table.currentPid); else table.toggleCurrent()
                } else if (event.key === Qt.Key_F9 || t === "k") {
                    const it = list.currentItem
                    const p = it ? it.mapToItem(table, it.width / 3, it.height) : Qt.point(0, 0)
                    table.openMenu(p.x, p.y)
                } else if (event.key === Qt.Key_F7 || t === "]") {
                    table.renice(-1)
                } else if (event.key === Qt.Key_F8 || t === "[") {
                    table.renice(1)
                } else if (t === "U") {
                    table.tagged = ({})
                } else if (event.key === Qt.Key_Home) {
                    list.currentIndex = 0
                } else if (event.key === Qt.Key_End) {
                    list.currentIndex = rows.count - 1
                } else if (event.key === Qt.Key_PageDown) {
                    list.currentIndex = Math.min(rows.count - 1, list.currentIndex + Math.floor(list.height / table.rowHeight))
                } else if (event.key === Qt.Key_PageUp) {
                    list.currentIndex = Math.max(0, list.currentIndex - Math.floor(list.height / table.rowHeight))
                } else if (!(table.letterHandler && t.length === 1 && table.letterHandler(t))) {
                    return
                }
                event.accepted = true
            }
        }
    }

    Menu {
        id: columnMenu
        MenuItem { text: "Columns"; enabled: false }
        Repeater {
            model: table.columns.filter(c => c.key !== "command")
            MenuItem {
                text: modelData.title
                checkable: true
                checked: table.hiddenColumns.indexOf(modelData.key) < 0
                onTriggered: table.toggleColumn(modelData.key)
            }
        }
        MenuSeparator {}
        MenuItem {
            text: "Shorten /nix/store hashes"
            checkable: true
            checked: table.shortenStorePaths
            onTriggered: table.shortenStorePaths = !table.shortenStorePaths
        }
        MenuItem { text: "Show all columns"; onTriggered: table.hiddenColumns = [] }
    }

    Menu {
        id: menu
        readonly property var sigs: JSON.parse(table.monitor.signal_list())
        MenuItem {
            enabled: false
            text: {
                const p = table.selectedPids()
                return p.length > 1 ? p.length + " tagged processes" : (p.length === 1 ? "PID " + p[0] : "No process")
            }
        }
        MenuSeparator {}
        MenuItem { text: "Terminate (SIGTERM)"; icon.name: "process-stop"; onTriggered: table.signalSelected(15) }
        MenuItem { text: "Kill (SIGKILL)"; icon.name: "edit-delete"; onTriggered: table.signalSelected(9) }
        MenuItem { text: "Stop (SIGSTOP)"; icon.name: "media-playback-pause"; onTriggered: table.signalSelected(19) }
        MenuItem { text: "Continue (SIGCONT)"; icon.name: "media-playback-start"; onTriggered: table.signalSelected(18) }
        Menu {
            title: "Send signal"
            Repeater {
                model: menu.sigs
                MenuItem { text: "SIG" + modelData.name + "  (" + modelData.value + ")"; onTriggered: table.signalSelected(modelData.value) }
            }
        }
        MenuSeparator {}
        MenuItem { text: "Nice +1 (lower priority)"; onTriggered: table.renice(1) }
        MenuItem { text: "Nice −1 (higher priority)"; onTriggered: table.renice(-1) }
        MenuSeparator {}
        MenuItem { text: "Details…"; icon.name: "documentinfo"; enabled: table.currentPid > 0; onTriggered: table.showDetails(table.currentPid) }
        MenuItem { text: "Collapse / expand"; onTriggered: table.toggleCurrent() }
        MenuItem { text: "Expand all"; onTriggered: table.monitor.set_expanded_all(true) }
        MenuItem { text: "Collapse all"; onTriggered: table.monitor.set_expanded_all(false) }
        MenuItem { text: "Clear tags"; enabled: Object.keys(table.tagged).length > 0; onTriggered: table.tagged = ({}) }
    }
}
