import QtQuick
import QtQuick.Controls
import QtQuick.Controls.impl
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// A menu item laid out like KDE's own (room for the check box, then a
// monochrome icon, then the text) with its key shown at the right. The key
// is only a hint: the window's key handling does the work, so typing in the
// filter never triggers menu items.
MenuItem {
    id: item
    property string keys: ""
    contentItem: RowLayout {
        Item {
            Layout.preferredWidth: item.checkable ? item.indicator.width : Kirigami.Units.smallSpacing
        }
        // Drawn pixel-exact: at a fractional scale (4/3) a 16 pt icon is
        // 21.33 device pixels, so the 16 px symbol was stretched between
        // pixels and its strokes went soft. Instead an icon theme design size
        // (16/22/24/32 px) whose size in points is whole (Qt rounds
        // sourceSize to whole points, then multiplies by the scale), the one
        // nearest 16 pt, on the device pixel grid: at 4/3, 24 px (18 pt).
        // (In a slot of the usual size, so rows keep their height.)
        Item {
            Layout.alignment: Qt.AlignVCenter
            Layout.preferredWidth: Kirigami.Units.iconSizes.small
            Layout.preferredHeight: Kirigami.Units.iconSizes.small
        IconImage {
            id: rowIcon
            readonly property real size: Kirigami.Units.iconSizes.small
            readonly property real dpr: {
                const w = Window.window
                return w && w.devicePixelRatio > 0 ? w.devicePixelRatio : (Screen.devicePixelRatio || 1)
            }
            readonly property int px: {
                const want = size * dpr
                let best = 0
                for (const d of [16, 22, 24, 32]) {
                    const pt = d / dpr
                    if (Math.abs(pt - Math.round(pt)) > 0.01) continue
                    if (!best || Math.abs(d - want) < Math.abs(best - want)) best = d
                }
                return best
            }
            readonly property int pt: px > 0 ? Math.round(px / dpr) : Math.round(size)
            readonly property real shown: px > 0 ? px / dpr : size
            x: (size - shown) / 2
            y: (size - shown) / 2
            width: shown
            height: shown
            sourceSize: Qt.size(pt, pt)
            name: item.icon.name
            color: label.color
            transform: Translate { id: gridSnap }
            function snap() {
                if (!visible || px <= 0) return
                const p = mapToItem(null, 0, 0)
                const rx = p.x - gridSnap.x, ry = p.y - gridSnap.y
                gridSnap.x = Math.round(rx * dpr) / dpr - rx
                gridSnap.y = Math.round(ry * dpr) / dpr - ry
            }
            onXChanged: Qt.callLater(snap)
            onYChanged: Qt.callLater(snap)
            Connections {
                target: rowIcon.parent
                function onXChanged() { Qt.callLater(rowIcon.snap) }
                function onYChanged() { Qt.callLater(rowIcon.snap) }
            }
            onVisibleChanged: Qt.callLater(snap)
            onDprChanged: Qt.callLater(snap)
            // Placed again whenever the menu opens or moves.
            Connections {
                target: item.menu
                function onOpened() { Qt.callLater(rowIcon.snap) }
                function onXChanged() { Qt.callLater(rowIcon.snap) }
                function onYChanged() { Qt.callLater(rowIcon.snap) }
            }
        }
        }
        Label {
            id: label
            Layout.alignment: Qt.AlignVCenter
            Layout.fillWidth: true
            text: item.text
            font: item.font
            color: item.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
            elide: Text.ElideRight
        }
        Label {
            Layout.alignment: Qt.AlignVCenter
            visible: item.keys !== ""
            text: item.keys
            font: item.font
            color: label.color
            opacity: 0.6
        }
        Item {
            Layout.preferredWidth: Kirigami.Units.smallSpacing
        }
    }
}
