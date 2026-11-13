import QtQuick

// Filled history graph. `series` is [{ values: [...], color }]; the second
// series (if any) is drawn mirrored below the axis when `mirrored` is set,
// which is how the network/disk cards show in vs. out.
Canvas {
    id: c
    renderStrategy: Canvas.Threaded
    property var series: []
    property real maximum: 0        // 0 = autoscale
    property int capacity: 60
    property bool mirrored: false
    property color gridColor: "#40808080"
    readonly property real peak: {
        let m = 0
        for (const s of series) for (const v of s.values) m = Math.max(m, v)
        return m
    }
    onSeriesChanged: requestPaint()
    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()

    onPaint: {
        const ctx = getContext("2d")
        ctx.reset()
        const w = width, h = height
        const mx = maximum > 0 ? maximum : Math.max(1, peak * 1.15)
        const mid = mirrored ? h / 2 : h
        ctx.strokeStyle = gridColor
        ctx.lineWidth = 1
        ctx.beginPath(); ctx.moveTo(0, mid - 0.5); ctx.lineTo(w, mid - 0.5); ctx.stroke()
        const step = w / Math.max(1, capacity - 1)
        for (let si = 0; si < series.length; si++) {
            const s = series[si]
            const vals = s.values
            if (!vals || vals.length < 2) continue
            const down = mirrored && si === 1
            const span = mirrored ? h / 2 : h
            const x0 = w - (vals.length - 1) * step
            ctx.beginPath()
            ctx.moveTo(x0, mid)
            for (let i = 0; i < vals.length; i++) {
                const v = Math.min(1, vals[i] / mx) * (span - 1)
                ctx.lineTo(x0 + i * step, down ? mid + v : mid - v)
            }
            ctx.lineTo(x0 + (vals.length - 1) * step, mid)
            ctx.closePath()
            ctx.fillStyle = Qt.rgba(s.color.r, s.color.g, s.color.b, 0.28)
            ctx.fill()
            ctx.beginPath()
            for (let i = 0; i < vals.length; i++) {
                const v = Math.min(1, vals[i] / mx) * (span - 1)
                const y = down ? mid + v : mid - v
                if (i === 0) ctx.moveTo(x0, y); else ctx.lineTo(x0 + i * step, y)
            }
            ctx.strokeStyle = s.color
            ctx.lineWidth = 1.5
            ctx.stroke()
        }
    }
}
