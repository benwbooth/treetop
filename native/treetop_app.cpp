#include "treetop_app.h"

#include <QtCore/QTimer>
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
            "</interface>");
    }
    bool handleMessage(const QDBusMessage &message, const QDBusConnection &connection) override
    {
        const QString m = message.member();
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
