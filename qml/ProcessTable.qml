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
    // Restored once rows first arrive: the selection and scroll position
    // from last time.
    property string restoreKey: ""
    property real restoreY: 0
    property real restoreX: 0
    property bool restored: false
    readonly property real scrollY: list.contentY
    readonly property real scrollX: list.contentX
    signal scrollMoved()
    function toggleFocus(pid) {
        if (pid > 0) monitor.set_focus(focusPid === pid ? 0 : pid)
    }
    property var hiddenColumns: []
    property bool shortenStorePaths: true
    // How /nix/store paths read in rows: "full", "hash" (/nix/store/abcde…-)
    // or "compact" (/n…/s…/abcde…-).
    property string storeStyle: "compact"
    onStoreStyleChanged: { maxLineChars = 0; clearSynCache() }
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
        { key: "ior", title: "DISK R", chars: 8, right: true,
          tip: "Disk reads per second by this process, as the storage layer counts them (data actually fetched from disk, not page-cache hits). Blank for other users' processes: the kernel only shows their I/O counters to root." },
        { key: "iow", title: "DISK W", chars: 8, right: true,
          tip: "Disk writes per second by this process (data it caused to be written to storage). Blank for other users' processes, whose counters only root can read." },
        { key: "net", title: "NET ↓/↑", chars: 15, right: true,
          tip: "Network download/upload per second. Linux keeps no per-process network counters (tools like nethogs need root to capture packets), but it does count per network namespace — so this shows each container's traffic, on the container's first process and on its row in the Containers view. Host processes share the host's network and stay blank; the Network card above shows the host total." },
        { key: "time", title: "TIME+", chars: 10, right: true,
          tip: "Total CPU time the process has used since it started (user + system), in minutes:seconds.hundredths, or hours once it passes an hour. A busy process accumulates time quickly; a long-lived idle one barely moves." },
        { key: "container", title: "CONTAINER", chars: 16, optional: true,
          tip: "The Docker/Podman/containerd container the process belongs to, worked out from its cgroup. Container processes also get a coloured stripe, and the first process of each container a badge, in the Command column." },
        { key: "command", title: "Command", chars: 0,
          tip: "The full command line. In tree view the lines show which process started which; ▸/▾ collapse and expand children (or +/−). The program name is in bold." },
    ]
    property bool showContainerColumn: true
    readonly property var forcedColumns: view === "gpu" ? ["gpu", "vram", "gpus", "engines"] : []
    // Column widths you set (key -> characters), applied over the defaults.
    property var widthOverride: ({})
    readonly property var columnsEff: columns.map(c => c.chars > 0 && widthOverride[c.key]
        ? Object.assign({}, c, { chars: widthOverride[c.key] }) : c)
    readonly property var visibleColumns: columnsEff.filter(c =>
        c.key === "command" || forcedColumns.indexOf(c.key) >= 0
        || ((!c.optional || showContainerColumn) && hiddenColumns.indexOf(c.key) < 0))

    function toggleColumn(key) {
        const h = hiddenColumns.slice()
        const i = h.indexOf(key)
        if (i >= 0) h.splice(i, 1); else h.push(key)
        hiddenColumns = h
    }

    // /nix/store/<32-char hash>-name -> /nix/store/<first 5>…-name: short,
    // but different builds of the same package stay distinguishable.
    readonly property var storeHash: /\/nix\/store\/([0-9a-df-np-sv-z]{5})[0-9a-df-np-sv-z]{27}-/g
    function tidy(s) {
        if (!shortenStorePaths || storeStyle === "full") return s
        return s.replace(storeHash, storeStyle === "compact" ? "/n…/s…/$1…-" : "/nix/store/$1…-")
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
        const group = r.group, pid = r.pid, user = r.user, pri = r.pri, ni = r.ni, virt = r.virt, res = r.res, shr = r.shr
        const state = r.state, cpu = r.cpu, mem = r.mem, gpus = r.gpus, gpu = r.gpu, vram = r.vram, gpuEngines = r.gpuEngines
        const ioR = r.ioR, ioW = r.ioW, netRx = r.netRx, netTx = r.netTx, count = r.count, time = r.time, container = r.container
        switch (k) {
        case "pid": return group ? "" : String(pid)
        case "user": return user
        case "pri": return group ? "" : (pri <= -100 ? "RT" : String(pri))
        case "ni": return group ? "" : String(ni)
        case "virt": return group ? "" : Fmt.bytes(virt)
        case "res": return Fmt.bytes(res)
        case "shr": return group ? "" : Fmt.bytes(shr)
        case "state": return state
        // Blank instead of 0.0, so active processes stand out.
        case "cpu": return cpu >= 0.05 ? cpu.toFixed(1) : ""
        case "mem": return mem >= 0.05 ? mem.toFixed(1) : ""
        case "gpu": return gpus.length > 0 || gpu > 0 ? gpu.toFixed(1) : ""
        case "vram": return vram > 0 ? Fmt.bytes(vram) : ""
        case "gpus": return gpus
        case "engines": return gpuEngines
        case "ior": return ioR > 0 ? Fmt.bytes(ioR) : (ioR < 0 ? "" : "0")
        case "iow": return ioW > 0 ? Fmt.bytes(ioW) : (ioW < 0 ? "" : "0")
        case "net": return netRx < 0 ? "" : Fmt.bytes(netRx) + "/" + Fmt.bytes(netTx)
        case "time": return group ? count + (count === 1 ? " proc" : " procs") : Fmt.cpuTime(time)
        case "container": return group ? "" : container
        }
        return ""
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
    readonly property var syn: darkBase ? {
        dir: Qt.rgba(textColor.r, textColor.g, textColor.b, 0.45), prog: "#7fe08f",
        flag: "#56b6e0", eq: Qt.rgba(textColor.r, textColor.g, textColor.b, 0.45), value: "#e3c25a",
        path: "#7f97c9", leaf: "#a9c1f2", number: "#c98ae0", url: "#e8995a", plain: textColor,
    } : {
        dir: Qt.rgba(textColor.r, textColor.g, textColor.b, 0.5), prog: "#17803a",
        flag: "#16709a", eq: Qt.rgba(textColor.r, textColor.g, textColor.b, 0.5), value: "#8f6a00",
        path: "#3f5d99", leaf: "#203f80", number: "#86379f", url: "#a8541a", plain: textColor,
    }
    // Plain object mutated in place: filling it mustn't notify bindings.
    readonly property var synCache: ({ map: {}, size: 0 })
    function clearSynCache() { synCache.map = {}; synCache.size = 0 }
    onSynChanged: clearSynCache()

    function span(color, text, bold) {
        if (!text) return ""
        const t = Fmt.escape(text)
        return "<font color='" + color + "'>" + (bold ? "<b>" + t + "</b>" : t) + "</font>"
    }
    function pathHtml(tok) {
        const i = tok.lastIndexOf("/")
        if (i < 0 || i === tok.length - 1) return span(syn.path, tok)
        return span(syn.path, tok.slice(0, i + 1)) + span(syn.leaf, tok.slice(i + 1))
    }
    function valueHtml(v) {
        if (v.indexOf("://") > 0) return span(syn.url, v)
        if (v.indexOf("/") >= 0) return pathHtml(v)
        if (/^-?[0-9][0-9.,:xXa-fA-FkKmMgG]*$/.test(v)) return span(syn.number, v)
        return span(syn.value, v)
    }
    function argHtml(tok) {
        if (tok.startsWith("-") && tok.length > 1 && !/^-[0-9]/.test(tok)) {
            const eq = tok.indexOf("=")
            if (eq > 0) return span(syn.flag, tok.slice(0, eq)) + span(syn.eq, "=") + valueHtml(tok.slice(eq + 1))
            return span(syn.flag, tok)
        }
        const eq = tok.indexOf("=")
        if (eq > 0 && /^[A-Za-z_][A-Za-z0-9_.-]*$/.test(tok.slice(0, eq)))
            return span(syn.flag, tok.slice(0, eq)) + span(syn.eq, "=") + valueHtml(tok.slice(eq + 1))
        if (tok.indexOf("://") > 0) return span(syn.url, tok)
        if (tok.indexOf("/") >= 0) return pathHtml(tok)
        if (/^-?[0-9][0-9.,:]*$/.test(tok)) return span(syn.number, tok)
        if (tok.startsWith("[") && tok.endsWith("]")) return span(syn.dir, tok)   // kernel threads
        return span(syn.plain, tok)
    }
    // Program dir dim, program name bold, then each argument coloured by
    // kind: flags, values, paths, numbers, URLs. Cached per command line.
    // In-row text is capped (the tooltip has the whole thing): laying out
    // multi-kilobyte rich text for every row that scrolls in stutters.
    readonly property int maxRowCommand: 500
    function highlight(command) {
        const hit = synCache.map[command]
        if (hit !== undefined) return hit
        let cmd = tidy(command)
        if (cmd.length > maxRowCommand) cmd = cmd.slice(0, maxRowCommand) + " …"
        const parts = cmd.split(/( +)/)
        const prog = parts[0] || ""
        const slash = prog.lastIndexOf("/")
        let out = span(syn.dir, prog.slice(0, slash + 1)) + span(syn.prog, prog.slice(slash + 1), true)
        for (let i = 1; i < parts.length; i++) {
            const p = parts[i]
            out += /^ +$/.test(p) ? p.replace(/ /g, "&nbsp;") : argHtml(p)
        }
        if (synCache.size > 5000) clearSynCache()
        synCache.map[command] = out
        synCache.size++
        return out
    }

    // The tooltip version: program on the first line, then one argument per
    // indented line, keeping "--flag value" pairs together.
    // (Unshortened: the tooltip shows the full /nix/store paths.)
    function prettyCommand(command, exe) {
        const toks = command.split(/ +/).filter(t => t.length)
        if (toks.length === 0) return ""
        // First line: the executable's full path (from /proc/<pid>/exe), with
        // the name it was run as when that differs.
        const guessed = exe.endsWith("?")
        const path = guessed ? exe.slice(0, -1) : exe
        const prog = path || toks[0], slash = prog.lastIndexOf("/")
        let first = span(syn.dir, prog.slice(0, slash + 1)) + span(syn.prog, prog.slice(slash + 1), true)
        if (guessed) first += span(syn.dir, "  (? found on PATH; the real binary isn't readable without root)")
        if (path && toks[0] !== path) first += span(syn.dir, "  as " + toks[0])
        const lines = [first]
        const indent = "&nbsp;&nbsp;&nbsp;&nbsp;"
        for (let i = 1; i < toks.length; i++) {
            const t = toks[i], next = toks[i + 1]
            const isFlag = t.startsWith("-") && t.length > 1 && !/^-[0-9]/.test(t) && t.indexOf("=") < 0
            if (isFlag && next !== undefined && !next.startsWith("-")) {
                lines.push(indent + argHtml(t) + "&nbsp;" + argHtml(next))
                i++
            } else {
                lines.push(indent + argHtml(t))
            }
        }
        // Shell-style line continuations, barely visible.
        const faint = Qt.rgba(textColor.r, textColor.g, textColor.b, 0.18)
        return "<span style='white-space:pre'>" + lines.join("&nbsp;" + span(faint, "\\") + "<br>") + "</span>"
    }

    ListModel { id: rows }
    property var keys: []   // mirror of rows' keys, for fast lookup
    // Longest command line seen (in characters): how far rows scroll sideways.
    property int maxLineChars: 0
    onViewChanged: maxLineChars = 0
    onShortenStorePathsChanged: { maxLineChars = 0; clearSynCache() }

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
            if (y < list.contentY) list.contentY = y
            else if (y + rowHeight > list.contentY + list.height) list.contentY = y + rowHeight - list.height
        }
    }
    readonly property bool userScrolling: list.moving || list.flicking || ksV.active || ksH.active

    function applyDiff(d) {
        applyDiffAnchored(d)
        if (!restored && d.count > 0) {
            // First rows: put back the selection and scroll from last time.
            const i = keys.indexOf(restoreKey)
            if (i >= 0) select(i, false)
            list.contentY = list.clampY(restoreY)
            list.contentX = list.clampX(restoreX)
            restored = true
        }
    }
    function applyDiffAnchored(d) {
        // Anchor on what you're looking at: the selected row if it's on
        // screen, otherwise the top visible row. It stays put on screen
        // when rows above it come or go. Nothing moves while you scroll.
        const scrolling = userScrolling
        const atTop = list.contentY <= 0
        let anchorKey = "", offset = 0
        if (!scrolling && !atTop) {
            const sel = keys.indexOf(currentKey)
            const selOffset = sel >= 0 ? sel * rowHeight - list.contentY : -1
            const top = Math.max(0, Math.min(keys.length - 1, Math.floor(list.contentY / rowHeight)))
            if (selOffset >= 0 && selOffset < list.height) { anchorKey = currentKey; offset = selOffset }
            else if (keys.length) { anchorKey = keys[top]; offset = top * rowHeight - list.contentY }
        }
        const oldSel = keys.indexOf(currentKey)

        const k = keys
        let longest = maxLineChars
        for (const [i, row] of d.changed) {
            if (i < rows.count) rows.set(i, row); else rows.append(row)
            k[i] = row.key
            const n = row.prefix.length + 2 + Math.min(maxRowCommand + 2, tidy(row.command).length) + (row.container ? row.container.length + 6 : 0)
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
            list.contentY = 0
        } else if (anchorKey !== "") {
            const i = k.indexOf(anchorKey)
            if (i >= 0) {
                const maxY = Math.max(0, rows.count * rowHeight - list.height)
                const y = Math.max(0, Math.min(maxY, i * rowHeight - offset))
                if (Math.abs(y - list.contentY) > 0.5) list.contentY = y
            }
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
                    ToolTip.visible: containsMouse && !pressed
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
                    if (aggCount > 0 && (k === "cpu" || k === "mem" || k === "res" || k === "gpu" || k === "vram" || k === "ior" || k === "iow" || k === "net")) return "#56b6e0"
                    if (k === "state" && state === "R") return "#43c96b"
                    if (k === "state" && state === "D") return "#e05a4f"
                    if (k === "state" && (state === "Z" || state === "T")) return "#d6a72e"
                    if (k === "cpu" && cpu >= 10) return Qt.lighter(table.heat(cpu / 100), 1.25)
                    if (k === "gpu" && gpu >= 10) return Qt.lighter(table.heat(gpu / 100), 1.25)
                    if (k === "mem" && mem >= 2) return Qt.lighter(table.heat(Math.sqrt(mem / 100)), 1.25)
                    if (k === "res" && res >= 1073741824) return "#4fb3d9"
                    if (k === "vram" && vram >= 1073741824) return "#c06fd8"
                    if ((k === "ior" && ioR >= 1048576) || (k === "iow" && ioW >= 1048576)) return "#4fb3d9"
                    if (k === "net" && netRx + netTx >= 1048576) return "#4f8fe0"
                    if (k === "user" && user === "root") return "#e0794f"
                    if (k === "container") return chip
                    return fg
                }
                function commandHtml() {
                    return current ? Fmt.escape(table.tidy(command).slice(0, table.maxRowCommand)) : table.highlight(command)
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
                    HoverHandler { id: cellHover }
                    readonly property var hoverCol: cellHover.hovered ? table.colAt(cellHover.point.position.x + 4) : null
                    readonly property string hoverFull: hoverCol ? String(rowItem.cell(hoverCol.key)) : ""
                    Loader {
                        active: fixedText.hoverCol !== null && fixedText.hoverFull.length > fixedText.hoverCol.chars
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
                            text: (rowItem.depth > 0 && table.view === "containers" && rowItem.prefix === "" ? "  " : "") + rowItem.prefix
                            font: table.font
                            color: rowItem.current ? table.highlightTextColor : table.dimColor
                            anchors.verticalCenter: parent.verticalCenter
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
                        ToolTip.visible: canToggle && expHover.hovered
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
                        text: "Σ" + rowItem.aggCount + " "
                        font.family: table.font.family
                        font.pointSize: table.font.pointSize
                        font.bold: true
                        color: rowItem.current ? table.highlightTextColor : "#56b6e0"
                        anchors.verticalCenter: parent.verticalCenter
                        HoverHandler { id: aggHover }
                        ToolTip.visible: aggHover.hovered
                        ToolTip.delay: 500
                        ToolTip.text: "Collapsed: CPU, memory, GPU, disk and network on this row are totals for these " + rowItem.aggCount + " processes (this one and everything below it)."
                    }
                    Text {
                        text: rowItem.group ? Fmt.escape(table.tidy(rowItem.command)) : rowItem.commandHtml()
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
                            onHoveredChanged: if (!hovered) tipGrace.restart()
                        }
                        Timer { id: tipGrace; interval: 350 }
                        Loader {
                            id: tipLoader
                            readonly property bool overTip: item ? item.hovered : false
                            active: (cmdHover.hovered || overTip || tipGrace.running) && !rowItem.group && rowItem.command.length > 0
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
                    const p = list.mapToItem(table, list.width / 3, (i + 1) * table.rowHeight - list.contentY)
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
