#pragma once

// Creates the QApplication, sets native text rendering and the desktop
// Quick Controls style, enforces a single instance (a second launch toggles
// the running window), loads the QML and runs the event loop.
int treetopRun(bool startHidden, bool toggle);
