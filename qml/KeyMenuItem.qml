import QtQuick
import QtQuick.Controls
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
        Kirigami.Icon {
            Layout.alignment: Qt.AlignVCenter
            Layout.preferredWidth: Kirigami.Units.iconSizes.small
            Layout.preferredHeight: Kirigami.Units.iconSizes.small
            source: item.icon.name
            color: label.color
            isMask: true
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
