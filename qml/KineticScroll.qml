import QtQuick

// Smooth, kinetic scrolling for any Flickable/ListView. Mouse wheels glide
// to an accumulated target (faster when spun quickly); touchpads follow the
// fingers and keep coasting when they lift. Qt only delivers wheel events
// on a handler's own axis, so use two: one default (vertical) and one with
// orientation: Qt.Horizontal. Shift+wheel on the vertical one scrolls
// sideways.
WheelHandler {
    id: k
    required property Flickable view
    property real step: 72          // px per wheel notch, vertical
    property real sideStep: 80      // px per notch sideways (tilt / Shift)
    property real touchGain: 1.8
    readonly property bool active: animY.running || animX.running || coast.running || fling.running

    target: null
    acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad

    property real targetY: 0
    property real targetX: 0
    property real lastWheel: 0
    property real tpLast: 0
    property real tpVx: 0
    property real tpVy: 0

    function clampY(y) { return Math.max(0, Math.min(y, Math.max(0, view.contentHeight - view.height))) }
    function clampX(x) { return Math.max(0, Math.min(x, Math.max(0, view.contentWidth - view.width))) }
    function stop() { animY.stop(); animX.stop(); coast.stop(); fling.stop(); tpVx = 0; tpVy = 0 }
    function glideY(d) {
        if (!animY.running) targetY = view.contentY
        targetY = clampY(targetY + d)
        animY.stop(); animY.from = view.contentY; animY.to = targetY; animY.start()
    }
    function glideX(d) {
        if (!animX.running) targetX = view.contentX
        targetX = clampX(targetX + d)
        animX.stop(); animX.from = view.contentX; animX.to = targetX; animX.start()
    }

    onWheel: (ev) => {
        const now = Date.now()
        // Touchpads: pixel deltas, a touchpad device, or angles that aren't
        // whole notches (high-resolution scrolling).
        const touch = ev.pixelDelta.x !== 0 || ev.pixelDelta.y !== 0
            || (ev.device && ev.device.type === PointerDevice.TouchPad)
            || (ev.angleDelta.x % 120 !== 0) || (ev.angleDelta.y % 120 !== 0)
        const gap = now - lastWheel
        const fast = gap < 40 ? 4 : (gap < 80 ? 2.8 : (gap < 160 ? 1.7 : 1))
        if (touch && (ev.pixelDelta.x || ev.pixelDelta.y || ev.angleDelta.x || ev.angleDelta.y)) {
            animY.stop(); animX.stop(); coast.stop()
            view.cancelFlick()
            const px = ev.pixelDelta.x !== 0 || ev.pixelDelta.y !== 0
            let dx = -(px ? ev.pixelDelta.x : ev.angleDelta.x / 2) * touchGain
            let dy = -(px ? ev.pixelDelta.y : ev.angleDelta.y / 2) * touchGain
            if (ev.modifiers & Qt.ShiftModifier) { dx = dx || dy; dy = 0 }
            const dt = Math.max(1, now - tpLast)
            tpVx = tpVx * 0.6 + (dx / dt * 1000) * 0.4
            tpVy = tpVy * 0.6 + (dy / dt * 1000) * 0.4
            tpLast = now
            if (dx !== 0) view.contentX = clampX(view.contentX + dx)
            if (dy !== 0) view.contentY = clampY(view.contentY + dy)
            fling.restart()
        } else if (ev.angleDelta.x !== 0 || ((ev.modifiers & Qt.ShiftModifier) && ev.angleDelta.y !== 0)) {
            coast.stop()
            lastWheel = now
            glideX(-(ev.angleDelta.x !== 0 ? ev.angleDelta.x : ev.angleDelta.y) / 120 * sideStep * fast)
        } else if (ev.angleDelta.y !== 0) {
            coast.stop()
            lastWheel = now
            glideY(-ev.angleDelta.y / 120 * step * fast)
        }
        ev.accepted = true
    }

    property NumberAnimation animY: NumberAnimation {
        target: k.view; property: "contentY"; duration: 110; easing.type: Easing.OutCubic
    }
    property NumberAnimation animX: NumberAnimation {
        target: k.view; property: "contentX"; duration: 110; easing.type: Easing.OutCubic
    }
    // No touchpad events for a moment: the fingers lifted, so coast.
    property Timer fling: Timer {
        interval: 40
        onTriggered: {
            k.coast.vx = Math.abs(k.tpVx) > 120 ? k.tpVx * 1.2 : 0
            k.coast.vy = Math.abs(k.tpVy) > 120 ? k.tpVy * 1.2 : 0
            if (k.coast.vx !== 0 || k.coast.vy !== 0) k.coast.start()
            k.tpVx = 0
            k.tpVy = 0
        }
    }
    // Per-frame coasting with exponential decay, on both axes.
    property FrameAnimation coast: FrameAnimation {
        property real vx: 0
        property real vy: 0
        onTriggered: {
            const dt = Math.min(0.05, frameTime)
            const decay = Math.exp(-dt / 0.5)
            if (vx !== 0) {
                const x = k.clampX(k.view.contentX + vx * dt)
                if (x === k.view.contentX) vx = 0; else k.view.contentX = x
                vx *= decay
                if (Math.abs(vx) < 12) vx = 0
            }
            if (vy !== 0) {
                const y = k.clampY(k.view.contentY + vy * dt)
                if (y === k.view.contentY) vy = 0; else k.view.contentY = y
                vy *= decay
                if (Math.abs(vy) < 12) vy = 0
            }
            if (vx === 0 && vy === 0) stop()
        }
    }
}
