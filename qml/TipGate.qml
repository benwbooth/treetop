import QtQuick

// Tooltips only after the pointer really moved over us. Qt's "hovered"
// can stay true with nothing under the pointer (after a menu or dialog
// closes, when the window hides to the tray or loses focus), which popped
// tooltips up on their own. Put this directly in a component's root item:
// a handler on an ancestor sees the pointer over everything inside it
// without taking hover away from anything (an item stacked on top would).
// Require `armed` in each ToolTip.visible; call disarm() when a popup closes.
HoverHandler {
    id: gate
    property bool armed: false
    function disarm() { armed = false }
    onPointChanged: armed = true
    onHoveredChanged: if (!hovered) armed = false
    readonly property Connections watch: Connections {
        target: gate.parent ? gate.parent.Window.window : null
        function onActiveChanged() { gate.armed = false }
        function onVisibleChanged() { gate.armed = false }
    }
}
