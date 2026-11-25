import QtQuick

// Minimize, maximize/restore and close at the right end of the toolbar: the
// shared ChromeButtons (hearth/shared/app-menus), drawn as arbor draws
// Breeze's on its title bars, the same in all of Ben's apps.
ChromeButtons {
    required property color textColor
    required property color chromeColor
    // (Kept for the callers; the shared buttons use the text colour.)
    property color hoverColor
    property color dividerColor
    fg: textColor
    bg: chromeColor
}
