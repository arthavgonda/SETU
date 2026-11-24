#include "daemonclient.h"
#include "i18n.h"
#include "settings.h"
#include "setucore.h"

#include <QGuiApplication>
#include <QLocalServer>
#include <QLocalSocket>
#include <QQmlApplicationEngine>
#include <QQmlEngineExtensionPlugin>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QSGRendererInterface>

Q_IMPORT_QML_PLUGIN(Setu_CorePlugin)
Q_IMPORT_QML_PLUGIN(Setu_ThemePlugin)
Q_IMPORT_QML_PLUGIN(LedgerPlugin)
Q_IMPORT_QML_PLUGIN(Setu_ShellPlugin)

static void choosePlatform()
{
#ifdef Q_OS_LINUX
    if (!qEnvironmentVariableIsEmpty("QT_QPA_PLATFORM"))
        return;
    const QString session = qEnvironmentVariable("XDG_SESSION_TYPE");
    const QString desktop = qEnvironmentVariable("XDG_CURRENT_DESKTOP").toLower();
    if (session == QLatin1String("wayland") && desktop.contains(QLatin1String("gnome")) && !qEnvironmentVariableIsEmpty("DISPLAY"))
        qputenv("QT_QPA_PLATFORM", "xcb");
#endif
}

static void chooseRendering()
{
#ifdef Q_OS_LINUX
    if (qEnvironmentVariableIsEmpty("QT_QPA_PLATFORMTHEME"))
        qputenv("QT_QPA_PLATFORMTHEME", "generic");
#endif
    if (qEnvironmentVariableIsEmpty("SETU_SHELL_GPU") && qEnvironmentVariableIsEmpty("QT_QUICK_BACKEND"))
        QQuickWindow::setGraphicsApi(QSGRendererInterface::Software);
}

int main(int argc, char *argv[])
{
    choosePlatform();
    chooseRendering();
    QGuiApplication app(argc, argv);
    app.setQuitOnLastWindowClosed(false);
    setu::prepare(app, QStringLiteral("setu-shell"));
    QQuickStyle::setStyle(QStringLiteral("Basic"));

    const QString lockName = QStringLiteral("setu-shell-") + qEnvironmentVariable("USER", qEnvironmentVariable("USERNAME"));
    {
        QLocalSocket probe;
        probe.connectToServer(lockName);
        if (probe.waitForConnected(300))
            return 0;
    }
    QLocalServer::removeServer(lockName);
    QLocalServer lock;
    lock.setSocketOptions(QLocalServer::UserAccessOption);
    lock.listen(lockName);

    DaemonClient daemon(QStringLiteral("shell"));
    if (qEnvironmentVariableIsEmpty("SETU_RUNTIME_DIR") && qEnvironmentVariableIsEmpty("SETU_UI_PREVIEW"))
        daemon.startWhenMissing(false);
    SettingsStore settings;
    Translator translator;

    QQmlApplicationEngine engine;
    setu::install(engine);
    QObject::connect(&engine, &QQmlApplicationEngine::objectCreationFailed, &app, [] { QCoreApplication::exit(1); }, Qt::QueuedConnection);
    engine.loadFromModule("Setu.Shell", "ShellMain");
    daemon.start();
    return app.exec();
}
