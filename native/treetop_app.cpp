#include "treetop_app.h"

#include <QtCore/QSettings>
#include <QtCore/QTimer>
#include <QtGui/QScreen>
#include <QtGui/QWindow>
#if defined(TREETOP_WAYLAND_SESSION_RESTORE) && QT_CONFIG(wayland)
#include <qpa/qplatformwindow_p.h>
#endif
#include <algorithm>
#include <QtDBus/QDBusConnection>
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

#include <unistd.h>

// com.benwbooth.treetop on the session bus: Toggle() / Show(), so the
// panel widget and tray can show/hide the window without needing treetop
// on PATH. A virtual object, so no moc is needed.
namespace {
class TreetopBus : public QDBusVirtualObject
{
public:
    explicit TreetopBus(QObject *root) : m_root(root) {}
    QString introspect(const QString &) const override
    {
        return QStringLiteral(
            "<interface name=\"com.benwbooth.treetop\">"
            "<method name=\"Toggle\"/><method name=\"Show\"/><method name=\"Hide\"/>"
            "<method name=\"Top\"><arg type=\"s\" direction=\"out\"/></method>"
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
    if (auto *window = qobject_cast<QWindow *>(root)) {
        auto *state = new WindowState(window);
        // QML decided whether to start visible (tray start, rebuild restart).
        if (root->property("wantVisible").toBool())
            state->showRestored();
    }

    static TreetopBus bus(root);
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
