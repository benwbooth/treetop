#include "treetop_app.h"

#include <QtCore/QDir>
#include <QtCore/QFile>
#include <QtCore/QSettings>
#include <QtCore/QStandardPaths>
#include <QtCore/QTimer>
#include <QtGui/QScreen>
#include <QtGui/QWindow>
#if defined(TREETOP_WAYLAND_SESSION_RESTORE) && QT_CONFIG(wayland)
#include <qpa/qplatformwindow_p.h>
#endif
#include <algorithm>
#include <QtDBus/QDBusConnection>
#include <QtDBus/QDBusConnectionInterface>
#include <QtDBus/QDBusMessage>
#include <QtDBus/QDBusVirtualObject>
#include <QtGui/QFontDatabase>
#include <QtGui/QIcon>
#include <QtNetwork/QLocalServer>
#include <QtNetwork/QLocalSocket>
#include <QtQml/QQmlApplicationEngine>
#include <QtQml/QQmlContext>
#include <QtQuick/QQuickWindow>
#include <QtQuickControls2/QQuickStyle>
#include <QtWidgets/QApplication>

#include <functional>
#include <unistd.h>

// com.benwbooth.treetop on the session bus: Toggle() / Show(), so the
// panel widget and tray can show/hide the window without needing treetop
// on PATH. A virtual object, so no moc is needed.
namespace {
class TreetopBus : public QDBusVirtualObject
{
public:
    TreetopBus(QObject *root, std::function<void(QRect)> savePlacement) : m_root(root), m_savePlacement(std::move(savePlacement)) {}
    QString introspect(const QString &) const override
    {
        return QStringLiteral(
            "<interface name=\"com.benwbooth.treetop\">"
            "<method name=\"Toggle\"/><method name=\"Show\"/><method name=\"Hide\"/>"
            "<method name=\"Top\"><arg type=\"s\" direction=\"out\"/></method>"
            "<method name=\"SavePlacement\"><arg type=\"d\" name=\"x\"/><arg type=\"d\" name=\"y\"/>"
            "<arg type=\"d\" name=\"width\"/><arg type=\"d\" name=\"height\"/></method>"
            "<method name=\"SetMinimized\"><arg type=\"b\" name=\"minimized\"/></method>"
            "</interface>");
    }
    bool handleMessage(const QDBusMessage &message, const QDBusConnection &connection) override
    {
        const QString m = message.member();
        if (m == QLatin1String("Top")) {
            // Top users of each resource, as JSON (for the panel widget).
            QVariant json;
            QMetaObject::invokeMethod(m_root, "topJson", Qt::DirectConnection, Q_RETURN_ARG(QVariant, json));
            connection.send(message.createReply(json.toString()));
            return true;
        }
        if (m == QLatin1String("SavePlacement")) {
            // From our KWin script: where the window is (frame geometry).
            const QVariantList a = message.arguments();
            if (a.size() == 4 && m_savePlacement)
                m_savePlacement(QRect(qRound(a[0].toDouble()), qRound(a[1].toDouble()), qRound(a[2].toDouble()), qRound(a[3].toDouble())));
            connection.send(message.createReply());
            return true;
        }
        if (m == QLatin1String("SetMinimized")) {
            const QVariantList a = message.arguments();
            if (!a.isEmpty()) m_root->setProperty("minimizedByWm", a.constFirst().toBool());
            connection.send(message.createReply());
            return true;
        }
        const char *slot = m == QLatin1String("Toggle") ? "toggleWindow"
                         : m == QLatin1String("Show") ? "showWindow"
                         : m == QLatin1String("Hide") ? "hideWindow" : nullptr;
        if (!slot) return false;
        QMetaObject::invokeMethod(m_root, slot, Qt::QueuedConnection);
        connection.send(message.createReply());
        return true;
    }
private:
    QObject *m_root;
    std::function<void(QRect)> m_savePlacement;
};

// Where the window goes on Wayland. An app can neither read nor set its
// position there, and every hide to the tray makes a new toplevel that KWin
// places afresh (and this KWin offers no session-restore protocol). So on
// KDE treetop loads a small KWin script: it reports the window's frame
// geometry over D-Bus as it moves, and puts the window back there whenever
// it appears - after a hide, or when treetop starts again.
class KWinPlacement
{
public:
    void load()
    {
        if (!QGuiApplication::platformName().startsWith(QStringLiteral("wayland"))
            || !QDBusConnection::sessionBus().interface()->isServiceRegistered(QStringLiteral("org.kde.KWin")))
            return;
        QSettings s;
        const QRect r = s.value(QStringLiteral("MainWindow/frameGeometry")).toRect();
        const QString saved = r.isValid()
            ? QStringLiteral("{x: %1, y: %2, width: %3, height: %4}").arg(r.x()).arg(r.y()).arg(r.width()).arg(r.height())
            : QStringLiteral("null");
        const QString dir = QStandardPaths::writableLocation(QStandardPaths::AppDataLocation);
        QDir().mkpath(dir);
        m_path = dir + QStringLiteral("/kwin-placement.js");
        QFile f(m_path);
        if (!f.open(QIODevice::WriteOnly | QIODevice::Truncate)) return;
        f.write(QStringLiteral(R"JS(// Written by treetop: keeps its window where you left it (see treetop_app.cpp).
let last = %1;
function ours(w) { return w.resourceClass === "com.benwbooth.treetop" && w.normalWindow && !w.transient; }
function report(w) {
    if (w.deleted || w.minimized || w.fullScreen || w.maximizeMode !== 0) return;
    const g = w.frameGeometry;
    // Maximizing moves the frame before maximizeMode says so: a frame that
    // fills the maximize area isn't a place to go back to.
    const m = workspace.clientArea(KWin.MaximizeArea, w);
    if (Math.abs(g.x - m.x) < 2 && Math.abs(g.y - m.y) < 2 && Math.abs(g.width - m.width) < 2 && Math.abs(g.height - m.height) < 2) return;
    last = {x: g.x, y: g.y, width: g.width, height: g.height};
    callDBus("com.benwbooth.treetop", "/treetop", "com.benwbooth.treetop", "SavePlacement", g.x, g.y, g.width, g.height);
}
function watch(w, place) {
    if (!ours(w)) return;
    if (place && last && w.maximizeMode === 0) w.frameGeometry = {x: last.x, y: last.y, width: last.width, height: last.height};
    w.frameGeometryChanged.connect(() => report(w));
    // Wayland never tells an app it was minimized; tell treetop, so a click
    // on its tray icon or widget brings it back instead of "hiding" it.
    const minimized = () => callDBus("com.benwbooth.treetop", "/treetop", "com.benwbooth.treetop", "SetMinimized", w.minimized);
    w.minimizedChanged.connect(minimized);
    minimized();
    // A toplevel re-shown maximized has no size of its own to go back to:
    // unmaximizing returns it to where it was before.
    let before = last;
    w.maximizedChanged.connect(() => {
        if (w.maximizeMode !== 0) before = last;
        else if (before) w.frameGeometry = {x: before.x, y: before.y, width: before.width, height: before.height};
    });
}
workspace.windowAdded.connect(w => watch(w, true));
for (const w of workspace.windowList()) watch(w, false);
)JS").arg(saved).toUtf8());
        f.close();
        unload();
        QDBusMessage reply = QDBusConnection::sessionBus().call(QDBusMessage::createMethodCall(
            QStringLiteral("org.kde.KWin"), QStringLiteral("/Scripting"), QStringLiteral("org.kde.kwin.Scripting"), QStringLiteral("loadScript"))
            << m_path << QString::fromLatin1(kName));
        if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().isEmpty() || reply.arguments().constFirst().toInt() < 0)
            return;
        // start() runs every loaded script not yet running. Not
        // Script<id>.run: KWin reuses the id of the script just unloaded,
        // and that path can still reach the old, dead one.
        QDBusConnection::sessionBus().call(QDBusMessage::createMethodCall(
            QStringLiteral("org.kde.KWin"), QStringLiteral("/Scripting"), QStringLiteral("org.kde.kwin.Scripting"), QStringLiteral("start")));
        m_loaded = true;
    }
    void unload()
    {
        QDBusConnection::sessionBus().call(QDBusMessage::createMethodCall(
            QStringLiteral("org.kde.KWin"), QStringLiteral("/Scripting"), QStringLiteral("org.kde.kwin.Scripting"), QStringLiteral("unloadScript"))
            << QString::fromLatin1(kName));
    }
    void save(const QRect &r)
    {
        if (!r.isValid()) return;
        m_pending = r;
        if (!m_timer) {
            m_timer = new QTimer(qApp);
            m_timer->setSingleShot(true);
            m_timer->setInterval(400);
            QObject::connect(m_timer, &QTimer::timeout, qApp, [this] {
                QSettings s;
                s.setValue(QStringLiteral("MainWindow/frameGeometry"), m_pending);
            });
        }
        m_timer->start();
    }
    bool loaded() const { return m_loaded; }
private:
    static constexpr const char *kName = "treetop-placement";
    QString m_path;
    QRect m_pending;
    QTimer *m_timer = nullptr;
    bool m_loaded = false;
};
}

// Remembers the window's size, maximized state and screen (QSettings,
// MainWindow group) and restores them; the position comes back through
// KWin's session restore on Wayland (Qt 6.11+), or setPosition elsewhere.
namespace {
class WindowState final : public QObject
{
public:
    explicit WindowState(QWindow *window) : QObject(window), m_window(window)
    {
        QSettings s;
        s.beginGroup(QStringLiteral("MainWindow"));
        QScreen *screen = window->screen();
        const QString screenName = s.value(QStringLiteral("screen")).toString();
        for (QScreen *c : QGuiApplication::screens())
            if (c->name() == screenName) screen = c;
        if (!screen) screen = QGuiApplication::primaryScreen();
        if (screen) window->setScreen(screen);
        m_normal = s.value(QStringLiteral("normalGeometry"), window->geometry()).toRect();
        m_maximized = s.value(QStringLiteral("maximized"), false).toBool();
        const bool known = s.contains(QStringLiteral("normalGeometry"));
        s.endGroup();
        if (screen) {
            const QRect a = screen->availableGeometry();
            m_normal.setWidth(std::clamp(m_normal.width(), std::min(window->minimumWidth(), a.width()), a.width()));
            m_normal.setHeight(std::clamp(m_normal.height(), std::min(window->minimumHeight(), a.height()), a.height()));
            m_normal.moveLeft(std::clamp(m_normal.x(), a.left(), a.right() - m_normal.width() + 1));
            m_normal.moveTop(std::clamp(m_normal.y(), a.top(), a.bottom() - m_normal.height() + 1));
        }
        window->resize(m_normal.size());
        if (!QGuiApplication::platformName().startsWith(QStringLiteral("wayland")) && known)
            window->setPosition(m_normal.topLeft());
#if defined(TREETOP_WAYLAND_SESSION_RESTORE) && QT_CONFIG(wayland)
        if (QGuiApplication::platformName().startsWith(QStringLiteral("wayland"))) {
            // The role must be set before show() creates the xdg_toplevel.
            window->create();
            if (auto *native = window->nativeInterface<QNativeInterface::Private::QWaylandWindow>())
                native->setSessionRestoreId(QStringLiteral("treetop-main"));
        }
#endif
        window->setProperty("restoreMaximized", m_maximized);
        m_timer.setSingleShot(true);
        m_timer.setInterval(300);
        connect(&m_timer, &QTimer::timeout, this, [this] { save(); });
        const auto later = [this] { m_timer.start(); };
        connect(window, &QWindow::xChanged, this, later);
        connect(window, &QWindow::yChanged, this, later);
        connect(window, &QWindow::widthChanged, this, later);
        connect(window, &QWindow::heightChanged, this, later);
        connect(window, &QWindow::screenChanged, this, later);
        connect(window, &QWindow::windowStateChanged, this, [this](Qt::WindowState st) {
            if (st == Qt::WindowMaximized || st == Qt::WindowNoState) {
                m_maximized = st == Qt::WindowMaximized;
                m_window->setProperty("restoreMaximized", m_maximized);
            }
            m_timer.start();
        });
        connect(window, &QWindow::visibleChanged, this, [this](bool v) { if (!v) save(); });
        connect(qApp, &QCoreApplication::aboutToQuit, this, [this] { save(); });
    }
    void showRestored() { if (m_maximized) m_window->showMaximized(); else m_window->showNormal(); }
private:
    void save()
    {
        m_timer.stop();
        if (m_window->windowState() == Qt::WindowNoState && !m_maximized && m_window->isVisible())
            m_normal = m_window->geometry();
        QSettings s;
        s.beginGroup(QStringLiteral("MainWindow"));
        s.setValue(QStringLiteral("normalGeometry"), m_normal);
        s.setValue(QStringLiteral("maximized"), m_maximized);
        if (m_window->screen()) s.setValue(QStringLiteral("screen"), m_window->screen()->name());
        s.endGroup();
        s.sync();
    }
    QWindow *m_window;
    QTimer m_timer;
    QRect m_normal;
    bool m_maximized = false;
};
}

static QString serverName()
{
    return QStringLiteral("treetop-%1").arg(getuid());
}

int treetopRun(bool startHidden, bool toggle)
{
    // Hinted, subpixel text that matches the rest of the KDE desktop instead
    // of Qt Quick's default distance-field glyphs. Must precede any window.
    QQuickWindow::setTextRenderType(QQuickWindow::NativeTextRendering);
    if (qEnvironmentVariableIsEmpty("QT_QUICK_CONTROLS_STYLE")) {
        QQuickStyle::setStyle(QStringLiteral("org.kde.desktop"));
    }
    QQuickStyle::setFallbackStyle(QStringLiteral("Fusion"));

    static int argc = 1;
    static char arg0[] = "treetop";
    static char *argv[] = {arg0, nullptr};
    QApplication app(argc, argv);
    QApplication::setApplicationName(QStringLiteral("treetop"));
    QApplication::setOrganizationName(QStringLiteral("treetop"));
    QApplication::setDesktopFileName(QStringLiteral("com.benwbooth.treetop"));
    QApplication::setQuitOnLastWindowClosed(false);
    QApplication::setWindowIcon(QIcon::fromTheme(QStringLiteral("treetop"),
        QIcon(QStringLiteral(":/qt/qml/com/benwbooth/treetop/qml/treetop.svg"))));

    {
        QLocalSocket probe;
        probe.connectToServer(serverName());
        if (probe.waitForConnected(300)) {
            // Already running. A login/background start (--hidden) leaves it
            // alone; a plain launch shows it; --toggle flips it.
            if (startHidden && !toggle) {
                return 0;
            }
            probe.write(toggle ? "toggle\n" : "show\n");
            probe.flush();
            probe.waitForBytesWritten(500);
            return 0;
        }
    }
    QLocalServer::removeServer(serverName());
    QLocalServer server;
    server.listen(serverName());

    QQmlApplicationEngine engine;
    // KDE's configured fixed-width font (System Settings > Fonts > Fixed width).
    engine.rootContext()->setContextProperty(QStringLiteral("systemFixedFont"),
        QFontDatabase::systemFont(QFontDatabase::FixedFont));
    engine.setInitialProperties({{QStringLiteral("startHidden"), startHidden}});
    engine.load(QUrl(QStringLiteral("qrc:/qt/qml/com/benwbooth/treetop/qml/Main.qml")));
    if (engine.rootObjects().isEmpty()) {
        return 1;
    }
    QObject *root = engine.rootObjects().constFirst();
    static KWinPlacement placement;
    // Before the first show, so the script sees the window appear.
    placement.load();
    QObject::connect(&app, &QCoreApplication::aboutToQuit, &app, [] { if (placement.loaded()) placement.unload(); });
    if (auto *window = qobject_cast<QWindow *>(root)) {
        auto *state = new WindowState(window);
        // QML decided whether to start visible (tray start, rebuild restart).
        if (root->property("wantVisible").toBool())
            state->showRestored();
    }

    static TreetopBus bus(root, [](QRect r) { placement.save(r); });
    QDBusConnection session = QDBusConnection::sessionBus();
    session.registerVirtualObject(QStringLiteral("/treetop"), &bus);
    session.registerService(QStringLiteral("com.benwbooth.treetop"));

    QObject::connect(&server, &QLocalServer::newConnection, &server, [&server, root] {
        while (QLocalSocket *s = server.nextPendingConnection()) {
            QObject::connect(s, &QLocalSocket::readyRead, s, [s, root] {
                const QByteArray cmd = s->readAll().trimmed();
                QMetaObject::invokeMethod(root, cmd == "toggle" ? "toggleWindow" : "showWindow");
                s->disconnectFromServer();
            });
            QObject::connect(s, &QLocalSocket::disconnected, s, &QObject::deleteLater);
        }
    });

    return app.exec();
}
