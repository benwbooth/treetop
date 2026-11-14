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
    // Focus mode: the PID whose tree is shown alone (0: everything).
    property int focusPid: 0
    // Tooltips stay out of the way of menus: hidden while a menu is open, and
    // for a moment after any click (so one can't pop up over a context menu).
    readonly property bool tipsBlocked: menu.opened || columnMenu.opened || tipHold.running || !tipGate.armed
    TipGate { id: tipGate }
    // The one row whose command tooltip may show; hovering anything else in
    // the table takes it away (a tooltip's own hover state can go stale).
    property string cmdTipKey: ""
    Timer { id: tipHold; interval: 1200 }
    function holdTips() { tipHold.restart() }
    // Restored once rows first arrive: the selection and scroll position
    // from last time.
    property string restoreKey: ""
    property real restoreY: 0
    property real restoreX: 0
    property bool restored: false
    // Scroll position relative to the list's origin. A ListView's scroll
    // range doesn't always start at 0 (originY moves as the model changes),
    // and treating 0 as the top parked the view where no rows exist.
    readonly property real scrollY: list.contentY - list.originY
    function relY() { return list.contentY - list.originY }
    function setRelY(y) {
        const maxY = Math.max(0, rows.count * rowHeight - list.height)
        list.contentY = list.originY + Math.max(0, Math.min(maxY, y))
    }
    readonly property real scrollX: list.contentX
    signal scrollMoved()
    function toggleFocus(pid) {
        if (pid > 0) monitor.set_focus(focusPid === pid ? 0 : pid)
    }
    property var hiddenColumns: []
    property bool shortenStorePaths: true
    // Resource columns lightly tinted in their graph colour (CPU green,
    // memory yellow, GPU purple, VRAM pink, disk/network blue).
    property bool tintColumns: true
    readonly property var tintColors: ({
        cpu: "#43c96b", time: "#43c96b",
        mem: "#d6b23e", res: "#d6b23e", virt: "#d6b23e", shr: "#d6b23e",
        gpu: "#c06fd8", vram: "#d070a8",
        ior: "#4fb3d9", iow: "#4fb3d9", net: "#4f8fe0"
    })
    readonly property var tintKeys: tintColumns ? fixedColumns.map(c => c.key).filter(k => tintColors[k] !== undefined) : []
    // How /nix/store paths read in rows: "full", "hash" (/nix/store/abcde…-)
    // or "compact" (/n…/s…/abcde…-).
    property string storeStyle: "compact"
    onStoreStyleChanged: maxLineChars = 0
    readonly property string effectiveStoreStyle: shortenStorePaths ? storeStyle : "full"
    onEffectiveStoreStyleChanged: if (monitor) monitor.set_store_style(effectiveStoreStyle)
    onMonitorChanged: if (monitor) {
        monitor.set_store_style(effectiveStoreStyle)
        monitor.set_palette(darkBase, String(textColor))
    }
    readonly property bool frozen: listHover.hovered
    signal notice(string text)
    signal showDetails(int pid)

    // One character's width as the row text is actually drawn. Native text
    // rendering snaps glyph advances to whole pixels, so the font's
    // fractional advance would let the header drift from the rows.
    readonly property real cw: cwProbe.implicitWidth > 0 ? cwProbe.implicitWidth / 100 : fm.advanceWidth("0")
    Text { id: cwProbe; visible: false; font: table.font; text: "0".repeat(100) }
    readonly property real rowHeight: Math.ceil(fm.height) + 3
    FontMetrics { id: fm; font: table.font }

    // The column table (key, title, width, tip...) comes from the shared
    // core, as do each row's cell texts and colours.
    readonly property var columns: monitor ? JSON.parse(monitor.columns()) : []
    // Column key -> its index in a row's cells/colors.
    readonly property var cellIndex: {
        const m = {}
        for (const c of columns) if (c.cell !== null && c.cell !== undefined) m[c.key] = c.cell
        return m
    }
    property bool showContainerColumn: true
    readonly property var forcedColumns: view === "gpu" ? ["gpu", "vram", "gpus", "engines"] : []
    // Column widths you set (key -> characters), applied over the defaults.
    property var widthOverride: ({})
    // Column order you set by dragging headers (keys); Command stays last.
    property var columnOrder: []
    readonly property var columnsEff: {
        const rank = (c, i) => c.key === "command" ? 1e9 : (columnOrder.indexOf(c.key) >= 0 ? columnOrder.indexOf(c.key) : 1e6 + i)
        return columns.map((c, i) => ({ c: c, r: rank(c, i) }))
            .sort((a, b) => a.r - b.r)
            .map(x => x.c.chars > 0 && widthOverride[x.c.key] ? Object.assign({}, x.c, { chars: widthOverride[x.c.key] }) : x.c)
    }
    // Moves a column so it sits before `beforeKey` ("" = at the end).
    function moveColumn(key, beforeKey) {
        if (key === beforeKey || key === "command") return
        const order = columnsEff.map(c => c.key).filter(k => k !== "command" && k !== key)
        const at = beforeKey ? order.indexOf(beforeKey) : -1
        if (at >= 0) order.splice(at, 0, key); else order.push(key)
        columnOrder = order
    }
    // Header drag state.
    property string dragKey: ""
    property real dragX: 0
    readonly property var dropTarget: {
        // {before: key or "", x}: where a dragged header would land.
        if (!dragKey) return null
        const cx = dragX + list.contentX
        for (const c of fixedColumns) {
            const st = columnStart[c.key]
            const left = 4 + st.at * cw, mid = left + (c.chars + 1) * cw / 2
            if (cx < mid) return { before: c.key, x: left - 2 }
        }
        return { before: "", x: 4 + fixedChars * cw - 2 }
    }
    readonly property var visibleColumns: columnsEff.filter(c =>
        c.key === "command" || forcedColumns.indexOf(c.key) >= 0
        || ((!c.optional || showContainerColumn) && hiddenColumns.indexOf(c.key) < 0))

    function toggleColumn(key) {
        const h = hiddenColumns.slice()
        const i = h.indexOf(key)
        if (i >= 0) h.splice(i, 1); else h.push(key)
        hiddenColumns = h
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
    // Column boundaries (x, before horizontal scrolling), in the gap
    // between one column's text and the next.
    readonly property var dividerXs: {
        const xs = []
        let n = 0
        for (const c of fixedColumns) { n += c.chars + 1; xs.push(4 + n * cw - cw * 0.5) }
        return xs
    }
    property color dividerColor: Qt.rgba(textColor.r, textColor.g, textColor.b, 0.10)
    // Where each fixed column starts (in characters), for the usage bars.
    readonly property var columnStart: {
        const m = {}
        let n = 0
        for (const c of fixedColumns) { m[c.key] = { at: n, chars: c.chars }; n += c.chars + 1 }
        return m
    }
    // A fixed column's text for a row (the delegate, or a ListModel row).
    function cellFor(r, k) {
        const i = cellIndex[k]
        if (i === undefined) return ""
        const arr = r.cellArr || String(r.cells || "").split("\u0001")
        return arr[i] || ""
    }

    // The fixed column under a row x position (content coordinates).
    function colAt(x) {
        const idx = Math.floor((x - 4) / cw)
        for (const c of fixedColumns) {
            const st = columnStart[c.key]
            if (st && idx >= st.at && idx < st.at + c.chars + 1) return c
        }
        return null
    }

    // Auto-fit: every fixed column as wide as its widest value or title.
    function autoFitColumns() {
        const w = {}
        for (const c of fixedColumns) w[c.key] = c.title.length + 1
        for (let i = 0; i < rows.count; i++) {
            const r = rows.get(i)
            for (const c of fixedColumns) {
                const n = String(cellFor(r, c.key)).length
                if (n > w[c.key]) w[c.key] = n
            }
        }
        const o = Object.assign({}, widthOverride)
        for (const k in w) o[k] = Math.max(2, Math.min(60, w[k]))
        widthOverride = o
    }
    function setColumnChars(key, chars) {
        const o = Object.assign({}, widthOverride)
        o[key] = Math.max(2, Math.min(80, Math.round(chars)))
        widthOverride = o
    }

    // Heat colour for a 0..1 load: green, then yellow, then red.
    function heat(f) {
        f = Math.max(0, Math.min(1, f))
        return f < 0.5 ? Qt.rgba(0.26 + f * 1.3, 0.79, 0.42 - f * 0.4, 1)
                       : Qt.rgba(0.91, 0.79 - (f - 0.5) * 1.1, 0.22 - (f - 0.5) * 0.2, 1)
    }

    // One row's fixed columns as a single monospace run (one Text item per
    // row instead of one per cell keeps scrolling cheap).
    function pad(t, c) {
        t = String(t)
        if (t.length > c.chars) t = t.slice(0, c.chars - 1) + "…"
        return (c.right ? t.padStart(c.chars) : t.padEnd(c.chars)) + " "
    }

    // ---- command-line highlighting ---------------------------------------
    readonly property bool darkBase: baseColor.hslLightness < 0.5
    // Command lines are coloured by the shared core; tell it the palette.
    readonly property string paletteKey: darkBase + "|" + textColor
    onPaletteKeyChanged: if (monitor) monitor.set_palette(darkBase, String(textColor))
    // In-row text is capped (the tooltip has the whole thing).
    readonly property int maxRowCommand: 500
    // The tooltip version: program on the first line, then one argument per
    // indented line (unshortened /nix/store paths).
    function prettyCommand(command, exe) {
        return monitor ? monitor.pretty_command(command, exe) : ""
    }

    ListModel { id: rows }
    property var keys: []   // mirror of rows' keys, for fast lookup
    // Longest command line seen (in characters): how far rows scroll sideways.
    property int maxLineChars: 0
    onViewChanged: maxLineChars = 0
    onShortenStorePathsChanged: maxLineChars = 0

    // The selection is our own (by process key), not ListView's
    // currentIndex: setting that makes ListView scroll to the row, which
    // yanked the view back while you scrolled elsewhere.
    function currentRow() { return keys.indexOf(currentKey) }
    function select(i, reveal) {
        if (i < 0 || i >= rows.count) return
        currentKey = keys[i]
        currentPid = rows.get(i).pid
        if (reveal) {
            // Scroll only as far as needed to show it.
            const y = i * rowHeight
            if (y < relY()) setRelY(y)
            else if (y + rowHeight > relY() + list.height) setRelY(y + rowHeight - list.height)
        }
    }
    readonly property bool userScrolling: list.moving || list.flicking || ksV.active || ksH.active

    function applyDiff(d) {
        applyDiffAnchored(d)
        if (!restored && d.count > 0) {
            // First rows: put back the selection and scroll from last time.
            const i = keys.indexOf(restoreKey)
            if (i >= 0) select(i, false)
            setRelY(restoreY)
            list.contentX = list.clampX(restoreX)
            restored = true
        }
    }
    function applyDiffAnchored(d) {
        // Anchor on what you're looking at: the selected row if it's on
        // screen, otherwise the top visible row. It stays put on screen
        // when rows above it come or go. Nothing moves while you scroll.
        const scrolling = userScrolling
        const ry = relY()
        const atTop = ry <= 0
        let anchorKey = "", offset = 0
        if (!scrolling && !atTop) {
            const sel = keys.indexOf(currentKey)
            const selOffset = sel >= 0 ? sel * rowHeight - ry : -1
            const top = Math.max(0, Math.min(keys.length - 1, Math.floor(ry / rowHeight)))
            if (selOffset >= 0 && selOffset < list.height) { anchorKey = currentKey; offset = selOffset }
            else if (keys.length) { anchorKey = keys[top]; offset = top * rowHeight - ry }
        }
        const oldSel = keys.indexOf(currentKey)

        const k = keys
        let longest = maxLineChars
        for (const [i, row] of d.changed) {
            if (i < rows.count) rows.set(i, row); else rows.append(row)
            k[i] = row.key
            const n = row.prefix.length + 2 + Math.min(maxRowCommand + 2, (row.cmdText || "").length) + (row.container ? row.container.length + 6 : 0)
            if (n > longest) longest = n
        }
        if (longest !== maxLineChars) maxLineChars = longest
        if (rows.count > d.count) rows.remove(d.count, rows.count - d.count)
        k.length = d.count

        if (d.count > 0 && (currentKey === "" || k.indexOf(currentKey) < 0)) {
            // Nothing selected yet, or the selected process exited: take the
            // row now on the same line (no scrolling).
            select(Math.min(Math.max(0, oldSel), d.count - 1), false)
        }
        if (atTop && !scrolling) {
            setRelY(0)
        } else if (anchorKey !== "") {
            const i = k.indexOf(anchorKey)
            if (i >= 0 && Math.abs(i * rowHeight - offset - relY()) > 0.5) setRelY(i * rowHeight - offset)
        }
    }

    // Safety net: if the list ever shows no rows while it has some (view
    // parked outside its content), lay it out again and go back to the
    // selection. Logged, so a recurrence can be diagnosed.
    Timer {
        interval: 700; repeat: true; running: rows.count > 0
        onTriggered: {
            if (table.userScrolling || rows.count === 0) return
            if (list.indexAt(10, list.contentY + 2) >= 0 || list.indexAt(10, list.contentY + list.height / 2) >= 0) return
            console.warn("treetop: list showed no rows; contentY", list.contentY, "originY", list.originY,
                         "contentHeight", list.contentHeight, "rows", rows.count, "— recovering")
            list.forceLayout()
            const i = Math.max(0, table.currentRow())
            table.setRelY(i * table.rowHeight - list.height / 3)
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
        if (currentPid <= 0 || currentRow() < 0) return
        const r = rows.get(currentRow())
        const nice = Math.max(-20, Math.min(19, r.ni + delta))
        notice(monitor.renice(selectedPids().join(","), nice))
    }

    function toggleCurrent() {
        if (currentRow() < 0) return
        const r = rows.get(currentRow())
        if (r.hasChildren) monitor.toggle(r.key)
    }

    function setCollapsed(collapsed) {
        if (currentRow() < 0) return
        const r = rows.get(currentRow())
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
            id: headerBar
            Layout.fillWidth: true
            implicitHeight: table.rowHeight + 3
            color: "#2e9e5b"
            // Safety: hovering the header with no button down ends any drag
            // whose release went missing.
            MouseArea {
                anchors.fill: parent
                z: -1
                hoverEnabled: true
                acceptedButtons: Qt.NoButton
                onPositionChanged: (m) => { if (table.dragKey && !(m.buttons & Qt.LeftButton)) table.dragKey = "" }
            }
            // Where a dragged column will land.
            Rectangle {
                z: 4
                visible: table.dropTarget !== null
                x: table.dropTarget ? table.dropTarget.x - list.contentX : 0
                width: 3
                height: parent.height
                color: "#e0c14f"
            }
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
                ToolTip.visible: colHover.hovered && !table.tipsBlocked
                ToolTip.delay: 500
                ToolTip.text: "Choose which columns to show (you can also right-click any column header)."
            }
            Repeater {
                model: table.dividerXs.length
                Rectangle {
                    x: table.dividerXs[index] - list.contentX
                    y: 3
                    width: 1
                    height: parent.height - 6
                    color: Qt.rgba(0, 0, 0, 0.25)
                }
            }
            // Column gaps: drag to resize the column on the left; double-click
            // to auto-fit every column.
            Repeater {
                model: table.dividerXs.length
                MouseArea {
                    z: 3
                    readonly property var col: table.fixedColumns[index]
                    x: table.dividerXs[index] - list.contentX - 5
                    width: 10
                    height: parent.height
                    cursorShape: Qt.SplitHCursor
                    hoverEnabled: true
                    preventStealing: true
                    onPositionChanged: (m) => {
                        if (!pressed || !col) return
                        const start = table.columnStart[col.key]
                        if (!start) return
                        // Pointer x in content coordinates, relative to the column's start.
                        const px = x + m.x + list.contentX - (4 + start.at * table.cw)
                        table.setColumnChars(col.key, px / table.cw - 0.5)
                    }
                    onDoubleClicked: table.autoFitColumns()
                    Rectangle {
                        anchors.centerIn: parent
                        width: 2
                        height: parent.height - 4
                        radius: 1
                        visible: parent.containsMouse || parent.pressed
                        color: "#0d1a12"
                    }
                    ToolTip.visible: containsMouse && !pressed && !table.tipsBlocked
                    ToolTip.delay: 800
                    ToolTip.text: "Drag to resize · double-click to fit all columns"
                }
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
                        HoverHandler { id: hh; onHoveredChanged: if (hovered) table.cmdTipKey = "" }
                        ToolTip.visible: hh.hovered && !table.tipsBlocked
                        ToolTip.delay: 600
                        ToolTip.timeout: 20000
                        ToolTip.text: "<b>" + modelData.title + "</b><br>" + modelData.tip
                            + "<br><br><i>Click to sort, drag to move, right-click to choose columns.</i>"
                        opacity: table.dragKey === modelData.key ? 0.45 : 1
                        // Click sorts; drag sideways to move the column.
                        MouseArea {
                            anchors.fill: parent
                            cursorShape: table.dragKey ? Qt.ClosedHandCursor : Qt.PointingHandCursor
                            acceptedButtons: Qt.LeftButton | Qt.RightButton
                            preventStealing: true
                            property real pressX: 0
                            property bool dragged: false
                            onPressed: (mouse) => { pressX = mouse.x; dragged = false }
                            onPositionChanged: (mouse) => {
                                if (!(pressed && (mouse.buttons & Qt.LeftButton)) || modelData.key === "command") return
                                if (!dragged && Math.abs(mouse.x - pressX) > 8) { dragged = true; table.dragKey = modelData.key }
                                if (dragged) table.dragX = mapToItem(headerBar, mouse.x, 0).x
                            }
                            onReleased: (mouse) => {
                                if (dragged) {
                                    const t = table.dropTarget
                                    if (t) table.moveColumn(modelData.key, t.before)
                                    table.dragKey = ""
                                }
                            }
                            onCanceled: table.dragKey = ""
                            // A cell rebuilt mid-drag never sees the release.
                            Component.onDestruction: if (table.dragKey === modelData.key) table.dragKey = ""
                            onClicked: (mouse) => {
                                if (dragged) return
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
            highlightFollowsCurrentItem: false
            keyNavigationEnabled: false
            ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }
            ScrollBar.horizontal: ScrollBar { policy: ScrollBar.AsNeeded }
            // Rows are as wide as the longest command line, and scroll sideways.
            contentWidth: Math.max(width, table.fixedWidth + 8 + (table.maxLineChars + 4) * table.cw)
            flickableDirection: Flickable.AutoFlickIfNeeded
            // Column dividers: a single overlay, so rows stay cheap.
            Item {
                parent: list
                anchors.fill: parent
                z: 5
                Repeater {
                    model: table.dividerXs.length
                    Rectangle {
                        x: table.dividerXs[index] - list.contentX
                        width: 1
                        height: parent.height
                        color: table.dividerColor
                    }
                }
            }
            onContentYChanged: table.scrollMoved()
            onContentXChanged: table.scrollMoved()
            HoverHandler {
                id: listHover
                onHoveredChanged: table.monitor.set_frozen(hovered)
            }
            flickDeceleration: 1400
            maximumFlickVelocity: 30000
            // Scroll speed: rows per wheel notch and touchpad gain.
            readonly property real wheelRows: 6
            readonly property real touchGain: 1.8

            // Smooth, kinetic scrolling (both axes; see KineticScroll.qml).
            KineticScroll {
                id: ksV
                view: list
                step: table.rowHeight * 6
                sideStep: table.cw * 10
            }
            KineticScroll {
                id: ksH
                view: list
                orientation: Qt.Horizontal
                sideStep: table.cw * 10
            }
            function clampY(y) { return ksV.clampY(y) }
            function clampX(x) { return ksV.clampX(x) }
            function glideX(dx) { ksV.glideX(dx) }

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
                required property string exe
                required property bool kernel
                required property int count
                required property bool containerRoot
                required property real gpu
                required property real vram
                required property string gpus
                required property string gpuEngines
                required property real ioR
                required property real ioW
                required property real netRx
                required property real netTx
                required property int aggCount
                required property string cells
                required property string colors
                required property string cmdHtml
                required property string cmdText
                readonly property var cellArr: cells.split("\u0001")
                readonly property var colorArr: colors.split("\u0001")

                readonly property bool current: key === table.currentKey
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

                function cell(k) { return table.cellFor(rowItem, k) }
                function cellColor(k) {
                    if (current) return table.highlightTextColor
                    const c = colorArr[table.cellIndex[k]]
                    return c ? c : fg
                }
                function commandHtml() {
                    return current ? Fmt.escape(cmdText.slice(0, table.maxRowCommand)) : cmdHtml
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

                Repeater {
                    model: table.tintKeys
                    Rectangle {
                        required property string modelData
                        readonly property var col: table.columnStart[modelData]
                        visible: col !== undefined && !rowItem.current
                        x: col ? 4 + (col.at - 0.4) * table.cw : 0
                        width: col ? (col.chars + 0.8) * table.cw : 0
                        height: rowItem.height
                        color: table.tintColors[modelData]
                        // Blue barely shows on a dark base; give it more.
                        opacity: modelData === "ior" || modelData === "iow" || modelData === "net" ? 0.14 : 0.08
                    }
                }
                // Usage bars behind CPU% / MEM% / GPU%, coloured by load, so
                // busy processes stand out at a glance.
                Repeater {
                    model: ["cpu", "mem", "gpu"]
                    Rectangle {
                        required property string modelData
                        readonly property var col: table.columnStart[modelData]
                        readonly property real v: modelData === "cpu" ? rowItem.cpu : (modelData === "mem" ? rowItem.mem : rowItem.gpu)
                        // CPU and GPU: 100% = full; memory: square-root scale,
                        // since a few percent of RAM is already a lot.
                        readonly property real frac: modelData === "mem" ? Math.sqrt(Math.max(0, v) / 100) : Math.max(0, v) / 100
                        visible: col !== undefined && v > 0.05
                        x: col ? 4 + col.at * table.cw : 0
                        y: 2
                        height: rowItem.height - 4
                        width: col ? Math.max(2, Math.min(1, frac) * col.chars * table.cw) : 0
                        radius: 2
                        color: table.heat(frac)
                        opacity: rowItem.current ? 0.55 : 0.32
                    }
                }
                Text {
                    id: fixedText
                    x: 4
                    width: table.fixedWidth
                    height: parent.height
                    verticalAlignment: Text.AlignVCenter
                    textFormat: Text.StyledText
                    text: rowItem.fixedHtml()
                    // A cell cut off with … shows its full text on hover.
                    HoverHandler { id: cellHover; onHoveredChanged: if (hovered) table.cmdTipKey = "" }
                    readonly property var hoverCol: cellHover.hovered ? table.colAt(cellHover.point.position.x + 4) : null
                    readonly property string hoverFull: hoverCol ? String(rowItem.cell(hoverCol.key)) : ""
                    Loader {
                        active: fixedText.hoverCol !== null && fixedText.hoverFull.length > fixedText.hoverCol.chars && !table.tipsBlocked
                        sourceComponent: ToolTip {
                            visible: true
                            delay: 400
                            x: cellHover.point.position.x + 12
                            y: rowItem.height + 2
                            text: fixedText.hoverCol ? fixedText.hoverCol.title + ": " + fixedText.hoverFull : ""
                            font: table.font
                        }
                    }
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
                    // Tree lines + ▸/▾: one big click target (the whole indent
                    // up to the name). Click toggles; Shift+click expands or
                    // collapses everything underneath.
                    Item {
                        readonly property bool canToggle: rowItem.hasChildren && table.view !== "flat" && table.view !== "gpu"
                        width: treeLines.implicitWidth + arrow.width
                        height: rowItem.height
                        Rectangle {
                            anchors.fill: parent
                            anchors.margins: 1
                            radius: 3
                            visible: parent.canToggle && expHover.hovered
                            color: Qt.rgba(0.34, 0.71, 0.88, 0.18)
                        }
                        Text {
                            id: treeLines
                            // htop's tree lines; container groups indent one level first.
                            // Box-drawing glyphs fill one text line, and a row is a
                            // little taller, which left gaps between rows: stretch
                            // just these to the full row height so they connect.
                            text: (rowItem.depth > 0 && table.view === "containers" && rowItem.prefix === "" ? "  " : "") + rowItem.prefix
                            font: table.font
                            color: rowItem.current ? table.highlightTextColor : table.dimColor
                            y: 0
                            transform: Scale { yScale: treeLines.implicitHeight > 0 ? rowItem.height / treeLines.implicitHeight : 1 }
                        }
                        Text {
                            id: arrow
                            x: treeLines.implicitWidth
                            width: table.cw * 2
                            text: parent.canToggle ? (rowItem.expanded ? "▾" : "▸") : ""
                            font.family: table.font.family
                            // A little larger than the text, to be easy to see and hit.
                            font.pointSize: table.font.pointSize * 1.35
                            horizontalAlignment: Text.AlignHCenter
                            // Same weight on hover (a bold ▾ may not exist in
                            // the font, which made it vanish); colour only.
                            color: expHover.hovered ? "#56b6e0" : rowItem.fg
                            anchors.verticalCenter: parent.verticalCenter
                        }
                        HoverHandler {
                            id: expHover
                            enabled: parent.canToggle
                            cursorShape: Qt.PointingHandCursor
                            onHoveredChanged: if (hovered) table.cmdTipKey = ""
                        }
                        TapHandler {
                            enabled: parent.canToggle
                            gesturePolicy: TapHandler.ReleaseWithinBounds
                            onTapped: (point) => {
                                if (point.modifiers & Qt.ShiftModifier)
                                    table.monitor.set_subtree_expanded(rowItem.key, !rowItem.expanded)
                                else
                                    table.monitor.toggle(rowItem.key)
                            }
                        }
                        ToolTip.visible: canToggle && expHover.hovered && !table.tipsBlocked
                        ToolTip.delay: 900
                        ToolTip.text: (rowItem.expanded ? "Collapse" : "Expand") + " · Shift+click: everything below · keys: ← → (+ −), Shift+← → scrolls"
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
                    // Collapsed branch: its numbers are the subtree's totals.
                    Text {
                        visible: rowItem.aggCount > 0
                        text: "×" + rowItem.aggCount + " "
                        font.family: table.font.family
                        font.pointSize: table.font.pointSize
                        font.bold: true
                        color: rowItem.current ? table.highlightTextColor : "#56b6e0"
                        anchors.verticalCenter: parent.verticalCenter
                        HoverHandler { id: aggHover; onHoveredChanged: if (hovered) table.cmdTipKey = "" }
                        ToolTip.visible: aggHover.hovered && !table.tipsBlocked
                        ToolTip.delay: 500
                        ToolTip.text: "Collapsed: CPU, memory, GPU, disk and network on this row are totals for these " + rowItem.aggCount + " processes (this one and everything below it)."
                    }
                    Text {
                        text: rowItem.group ? Fmt.escape(rowItem.cmdText) : rowItem.commandHtml()
                        textFormat: Text.StyledText
                        font: table.font
                        color: rowItem.group ? (rowItem.current ? table.highlightTextColor : table.dimColor) : rowItem.fg
                        anchors.verticalCenter: parent.verticalCenter
                        // Hover: the whole command (full paths), one argument per
                        // line, in the monospace font. Created only while hovered.
                        // Hover: the whole command (full paths), one argument
                        // per line. It opens beside the pointer (not on top of
                        // it), stays while the pointer is on the row or the
                        // tooltip, and closes only after a short grace period,
                        // so a tall tooltip can't flicker on and off.
                        HoverHandler {
                            id: cmdHover
                            onHoveredChanged: if (hovered) table.cmdTipKey = rowItem.key; else tipGrace.restart()
                        }
                        Timer { id: tipGrace; interval: 350 }
                        Loader {
                            id: tipLoader
                            readonly property bool overTip: item ? item.hovered : false
                            active: (cmdHover.hovered || overTip || tipGrace.running) && table.cmdTipKey === rowItem.key
                                && !rowItem.group && rowItem.command.length > 0 && !table.tipsBlocked
                            onOverTipChanged: if (!overTip) tipGrace.restart()
                            sourceComponent: ToolTip {
                                readonly property bool hovered: tipHover.hovered
                                visible: true
                                delay: 700
                                timeout: -1
                                font: table.font
                                // Beside and below the pointer.
                                x: cmdHover.point.position.x + 18
                                y: tipLoader.parent ? tipLoader.parent.height + 6 : 0
                                contentItem: Text {
                                    textFormat: Text.StyledText
                                    font: table.font
                                    color: table.textColor
                                    wrapMode: Text.WrapAnywhere
                                    text: table.prettyCommand(rowItem.command, rowItem.exe)
                                        + "<br><br><font color='" + table.dimColor + "'>PID " + rowItem.pid + " · " + Fmt.escape(rowItem.user)
                                        + (rowItem.container ? " · ⬢ " + Fmt.escape(rowItem.container) : "") + "</font>"
                                    HoverHandler { id: tipHover }
                                }
                            }
                        }
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
                        table.holdTips()
                        table.select(rowItem.index, false)
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
                        // Double-click: show only this process's tree (again: everything).
                        if (rowItem.hasChildren && rowItem.group) table.monitor.toggle(rowItem.key)
                        else table.toggleFocus(rowItem.pid)
                    }
                }
            }

            Keys.onPressed: (event) => {
                const t = event.text
                if (event.key === Qt.Key_Escape && table.dragKey) { table.dragKey = ""; event.accepted = true; return }
                if (event.key === Qt.Key_Space) {
                    if (table.currentPid > 0) {
                        const tg = Object.assign({}, table.tagged)
                        if (tg[table.currentPid]) delete tg[table.currentPid]; else tg[table.currentPid] = true
                        table.tagged = tg
                    }
                    table.select(table.currentRow() + 1, true)
                } else if ((event.modifiers & Qt.ShiftModifier) && event.key === Qt.Key_Right) {
                    list.glideX(table.cw * 8)
                } else if ((event.modifiers & Qt.ShiftModifier) && event.key === Qt.Key_Left) {
                    list.glideX(-table.cw * 8)
                } else if ((event.modifiers & Qt.ControlModifier) && (event.key === Qt.Key_Right || event.key === Qt.Key_Left)) {
                    // Ctrl+→ / Ctrl+←: expand / collapse the whole subtree.
                    const i = table.currentRow()
                    if (i >= 0) table.monitor.set_subtree_expanded(rows.get(i).key, event.key === Qt.Key_Right)
                } else if (t === "+" || t === "=" || event.key === Qt.Key_Right) {
                    table.setCollapsed(false)
                } else if (t === "-" || event.key === Qt.Key_Left) {
                    table.setCollapsed(true)
                } else if (t === "*") {
                    table.monitor.set_expanded_all(true)
                } else if (t === "_") {
                    table.monitor.set_expanded_all(false)
                } else if (t === "f") {
                    table.toggleFocus(table.currentPid)
                } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                    if (table.currentPid > 0) table.showDetails(table.currentPid); else table.toggleCurrent()
                } else if (event.key === Qt.Key_F9 || t === "k") {
                    const i = table.currentRow()
                    table.select(i, true)
                    const p = list.mapToItem(table, list.width / 3, (i + 1) * table.rowHeight - table.relY())
                    table.openMenu(p.x, p.y)
                } else if (event.key === Qt.Key_F7 || t === "]") {
                    table.renice(-1)
                } else if (event.key === Qt.Key_F8 || t === "[") {
                    table.renice(1)
                } else if (t === "U") {
                    table.tagged = ({})
                } else if (event.key === Qt.Key_Down) {
                    table.select(Math.min(rows.count - 1, table.currentRow() + 1), true)
                } else if (event.key === Qt.Key_Up) {
                    table.select(Math.max(0, table.currentRow() - 1), true)
                } else if (event.key === Qt.Key_Home) {
                    table.select(0, true)
                } else if (event.key === Qt.Key_End) {
                    table.select(rows.count - 1, true)
                } else if (event.key === Qt.Key_PageDown) {
                    table.select(Math.min(rows.count - 1, table.currentRow() + Math.floor(list.height / table.rowHeight)), true)
                } else if (event.key === Qt.Key_PageUp) {
                    table.select(Math.max(0, table.currentRow() - Math.floor(list.height / table.rowHeight)), true)
                } else if (!(table.letterHandler && t.length === 1 && table.letterHandler(t))) {
                    return
                }
                event.accepted = true
            }
        }
    }

    Menu {
        id: columnMenu
        onClosed: tipGate.disarm()
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
        Menu {
            title: "/nix/store paths"
            MenuItem { text: "Full"; checkable: true; checked: !table.shortenStorePaths || table.storeStyle === "full"; onTriggered: { table.shortenStorePaths = true; table.storeStyle = "full" } }
            MenuItem { text: "Short hash  (/nix/store/abcde…-)"; checkable: true; checked: table.shortenStorePaths && table.storeStyle === "hash"; onTriggered: { table.shortenStorePaths = true; table.storeStyle = "hash" } }
            MenuItem { text: "Compact  (/n…/s…/abcde…-)"; checkable: true; checked: table.shortenStorePaths && table.storeStyle === "compact"; onTriggered: { table.shortenStorePaths = true; table.storeStyle = "compact" } }
        }
        MenuItem { text: "Show all columns"; onTriggered: table.hiddenColumns = [] }
    }

    Menu {
        id: menu
        onClosed: tipGate.disarm()
        readonly property var sigs: JSON.parse(table.monitor.signal_list())
        MenuItem {
            enabled: false
            text: {
                const p = table.selectedPids()
                return p.length > 1 ? p.length + " tagged processes" : (p.length === 1 ? "PID " + p[0] : "No process")
            }
        }
        MenuSeparator {}
        MenuItem {
            text: table.focusPid > 0 && table.focusPid === table.currentPid ? "Show all processes (f)" : "Focus on this process tree (f)"
            icon.name: "zoom-select"
            enabled: table.currentPid > 0
            onTriggered: table.toggleFocus(table.currentPid)
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
        MenuItem { text: "Details… (Enter)"; icon.name: "documentinfo"; enabled: table.currentPid > 0; onTriggered: table.showDetails(table.currentPid) }
        MenuItem { text: "Collapse / expand"; onTriggered: table.toggleCurrent() }
        MenuItem { text: "Expand all"; onTriggered: table.monitor.set_expanded_all(true) }
        MenuItem { text: "Collapse all"; onTriggered: table.monitor.set_expanded_all(false) }
        MenuItem { text: "Clear tags"; enabled: Object.keys(table.tagged).length > 0; onTriggered: table.tagged = ({}) }
    }
}
