import QtQuick

// Tooltips only after the pointer really moved over us. Qt's "hovered"
// can stay true with nothing under the pointer (after a menu or dialog
// closes, when the window hides to the tray or loses focus), which popped
// tooltips up on their own. Fill the component with this and require
// `armed` in each ToolTip.visible; call disarm() when a popup closes.
Item {
    id: gate
    anchors.fill: parent
    // On top: controls underneath take hover events for themselves, so a
    // watcher below them never hears of the pointer. (It's passive: the
    // events still reach everything underneath.)
    z: 1000000
    property bool armed: false
    function disarm() { armed = false }
    HoverHandler {
        // Passive: it only watches; whatever is underneath still gets hovers.
        onPointChanged: gate.armed = true
        onHoveredChanged: if (!hovered) gate.armed = false
    }
    Connections {
        target: gate.Window.window
        function onActiveChanged() { gate.armed = false }
        function onVisibleChanged() { gate.armed = false }
    }
}
