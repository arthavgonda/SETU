#include "daemonclient.h"
#include "i18n.h"
#include "settings.h"
#include "setucore.h"

#include <QCommandLineParser>
#include <QGuiApplication>
#include <QLocalServer>
#include <QLocalSocket>
#include <QQmlApplicationEngine>
#include <QQmlEngineExtensionPlugin>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QTimer>

Q_IMPORT_QML_PLUGIN(Setu_CorePlugin)
Q_IMPORT_QML_PLUGIN(Setu_ThemePlugin)
Q_IMPORT_QML_PLUGIN(LedgerPlugin)
Q_IMPORT_QML_PLUGIN(Setu_AppPlugin)

class Router : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QString page READ page NOTIFY pageChanged)

public:
    explicit Router(const QString &page, QObject *parent = nullptr)
        : QObject(parent), m_page(page)
    {
    }
    QString page() const { return m_page; }
    Q_INVOKABLE void request(const QString &page)
    {
        m_page = page;
        emit pageChanged();
        emit raiseRequested();
    }

signals:
    void pageChanged();
    void raiseRequested();

private:
    QString m_page;
};

int main(int argc, char *argv[])
{
    QGuiApplication app(argc, argv);
    setu::prepare(app, QStringLiteral("setu-ui"));
    QQuickStyle::setStyle(QStringLiteral("Basic"));

    QCommandLineParser cli;
    cli.addHelpOption();
    QCommandLineOption pageOpt(QStringLiteral("page"), QStringLiteral("Page to open: wizard, home, settings/<page>"), QStringLiteral("page"), QStringLiteral("home"));
    cli.addOption(pageOpt);
    QCommandLineOption shotOpt(QStringLiteral("screenshot"), QStringLiteral("Save a PNG of the window and quit (for visual tests)"), QStringLiteral("file"));
    QCommandLineOption delayOpt(QStringLiteral("delay"), QStringLiteral("Milliseconds to wait before the screenshot"), QStringLiteral("ms"), QStringLiteral("2500"));
    QCommandLineOption sizeOpt(QStringLiteral("size"), QStringLiteral("Initial window size, WxH (for tests)"), QStringLiteral("size"));
    cli.addOption(shotOpt);
    cli.addOption(delayOpt);
    cli.addOption(sizeOpt);
    cli.process(app);
    const QString page = cli.value(pageOpt);
    const QString shot = cli.value(shotOpt);

    const QString lockName = QStringLiteral("setu-ui-") + qEnvironmentVariable("USER", qEnvironmentVariable("USERNAME")) + (shot.isEmpty() ? QString() : QStringLiteral("-shot-%1").arg(QCoreApplication::applicationPid()));
    {
        QLocalSocket probe;
        probe.connectToServer(lockName);
        if (probe.waitForConnected(300)) {
            probe.write(page.toUtf8());
            probe.flush();
            probe.waitForBytesWritten(300);
            return 0;
        }
    }
    QLocalServer::removeServer(lockName);
    QLocalServer server;
    server.setSocketOptions(QLocalServer::UserAccessOption);
    server.listen(lockName);

    DaemonClient daemon(QStringLiteral("window"));
    SettingsStore settings;
    Translator translator;
    Router router(page);

    QObject::connect(&server, &QLocalServer::newConnection, &app, [&] {
        while (QLocalSocket *s = server.nextPendingConnection()) {
            QObject::connect(s, &QLocalSocket::readyRead, s, [s, &router] {
                const QString p = QString::fromUtf8(s->readAll()).trimmed();
                if (!p.isEmpty())
                    router.request(p);
                s->deleteLater();
            });
        }
    });

    QQmlApplicationEngine engine;
    setu::install(engine);
    engine.setInitialProperties({{QStringLiteral("router"), QVariant::fromValue(&router)}});
    QObject::connect(&engine, &QQmlApplicationEngine::objectCreationFailed, &app, [] { QCoreApplication::exit(1); }, Qt::QueuedConnection);
    daemon.start();
    engine.loadFromModule("Setu.App", "AppMain");
    const QStringList wh = cli.value(sizeOpt).split(QLatin1Char('x'));
    if (wh.size() == 2) {
        auto *sizer = new QTimer(&app);
        sizer->setInterval(100);
        QObject::connect(sizer, &QTimer::timeout, &app, [sizer, wh] {
            for (QWindow *w : QGuiApplication::topLevelWindows()) {
                if (qobject_cast<QQuickWindow *>(w) && w->isVisible()) {
                    w->resize(wh[0].toInt(), wh[1].toInt());
                    sizer->stop();
                    return;
                }
            }
        });
        sizer->start();
    }
    if (!shot.isEmpty()) {
        QTimer::singleShot(cli.value(delayOpt).toInt(), &app, [shot, wh] {
            for (QWindow *w : QGuiApplication::topLevelWindows()) {
                auto *q = qobject_cast<QQuickWindow *>(w);
                if (!q || !q->isVisible())
                    continue;
                if (wh.size() == 2 && q->size() != QSize(wh[0].toInt(), wh[1].toInt())) {
                    q->resize(wh[0].toInt(), wh[1].toInt());
                    QCoreApplication::processEvents();
                }
                const bool ok = q->grabWindow().save(shot);
                QCoreApplication::exit(ok ? 0 : 2);
                return;
            }
            QCoreApplication::exit(3);
        });
    }
    return app.exec();
}

#include "main.moc"
