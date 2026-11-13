#include "launcher.h"

#include <QCoreApplication>
#include <QDir>
#include <QFileInfo>
#include <QFile>
#include <QProcess>
#include <QStandardPaths>

#ifdef Q_OS_UNIX
#include <unistd.h>
#endif

Launcher::Launcher(QObject *parent)
    : QObject(parent)
{
}

QString Launcher::binary(const QString &name)
{
#ifdef Q_OS_WIN
    const QString file = name + QStringLiteral(".exe");
#else
    const QString file = name;
#endif
    const QDir here(QCoreApplication::applicationDirPath());
    for (const QString &rel : {QStringLiteral("."), QStringLiteral("../daemon/target/release"), QStringLiteral("../../daemon/target/release")}) {
        const QString p = QDir::cleanPath(here.filePath(rel + QLatin1Char('/') + file));
        if (QFileInfo(p).isExecutable())
            return p;
    }
    return file;
}

bool Launcher::startDaemon()
{
#ifdef Q_OS_LINUX
    QProcess probe;
    probe.start(QStringLiteral("systemctl"), {QStringLiteral("--user"), QStringLiteral("cat"), QStringLiteral("setu-daemon.service")});
    if (probe.waitForFinished(1500) && probe.exitCode() == 0)
        return QProcess::startDetached(QStringLiteral("systemctl"), {QStringLiteral("--user"), QStringLiteral("start"), QStringLiteral("setu-daemon.service")});
#endif
    return QProcess::startDetached(binary(QStringLiteral("setu-daemon")), {});
}

bool Launcher::restartDaemon()
{
#if defined(Q_OS_LINUX)
    QProcess probe;
    probe.start(QStringLiteral("systemctl"), {QStringLiteral("--user"), QStringLiteral("cat"), QStringLiteral("setu-daemon.service")});
    if (probe.waitForFinished(1500) && probe.exitCode() == 0)
        return QProcess::startDetached(QStringLiteral("systemctl"), {QStringLiteral("--user"), QStringLiteral("restart"), QStringLiteral("setu-daemon.service")});
#elif defined(Q_OS_MACOS)
    return QProcess::startDetached(QStringLiteral("launchctl"), {QStringLiteral("kickstart"), QStringLiteral("-k"), QStringLiteral("gui/%1/ai.setu.daemon").arg(getuid())});
#endif
    return false;
}

bool Launcher::startShell()
{
#ifdef Q_OS_LINUX
    QProcess probe;
    probe.start(QStringLiteral("systemctl"), {QStringLiteral("--user"), QStringLiteral("is-enabled"), QStringLiteral("setu-shell.service")});
    if (probe.waitForFinished(1500) && probe.exitCode() == 0)
        return QProcess::startDetached(QStringLiteral("systemctl"), {QStringLiteral("--user"), QStringLiteral("start"), QStringLiteral("setu-shell.service")});
#endif
    return QProcess::startDetached(binary(QStringLiteral("setu-shell")), {});
}

bool Launcher::openWindow(const QString &page)
{
    return QProcess::startDetached(binary(QStringLiteral("setu-ui")), {QStringLiteral("--page"), page});
}

QString Launcher::logsFolder()
{
#if defined(Q_OS_MACOS)
    return QDir::homePath() + QStringLiteral("/Library/Logs/Setu");
#elif defined(Q_OS_WIN)
    return QStandardPaths::writableLocation(QStandardPaths::GenericDataLocation) + QStringLiteral("/Setu/logs");
#else
    const QString state = qEnvironmentVariable("XDG_STATE_HOME", QDir::homePath() + QStringLiteral("/.local/state"));
    return state + QStringLiteral("/setu/logs");
#endif
}

QString Launcher::lastDaemonError()
{
    QFile f(logsFolder() + QStringLiteral("/setu-daemon.log"));
    if (!f.open(QIODevice::ReadOnly))
        return {};
    if (f.size() > 65536)
        f.seek(f.size() - 65536);
    const QList<QByteArray> lines = f.readAll().split('\n');
    for (auto it = lines.crbegin(); it != lines.crend(); ++it) {
        const int at = it->indexOf(" ERROR ");
        if (at > 0)
            return QString::fromUtf8(it->mid(at + 7)).trimmed().left(300);
    }
    return {};
}

QString Launcher::installKind()
{
#if defined(Q_OS_WIN)
    return QStringLiteral("windows");
#elif defined(Q_OS_MACOS)
    return QStringLiteral("macos");
#else
    if (!qEnvironmentVariableIsEmpty("APPIMAGE"))
        return QStringLiteral("appimage");
    if (!qEnvironmentVariableIsEmpty("FLATPAK_ID"))
        return QStringLiteral("flatpak");
    const QString dir = QCoreApplication::applicationDirPath();
    if (dir.startsWith(QLatin1String("/usr/")) || dir.startsWith(QLatin1String("/opt/")))
        return QStringLiteral("package");
    return QStringLiteral("folder");
#endif
}

QString Launcher::uninstall()
{
#if defined(Q_OS_WIN)
    const QString exe = QDir(QCoreApplication::applicationDirPath()).filePath(QStringLiteral("uninstall.exe"));
    if (QFileInfo::exists(exe) && QProcess::startDetached(exe, {}))
        return {};
    return QStringLiteral("Open Settings > Apps > Installed apps and remove Setu there.");
#elif defined(Q_OS_MACOS)
    QDir bundle(QCoreApplication::applicationDirPath());
    bundle.cdUp();
    bundle.cdUp();
    if (bundle.dirName().endsWith(QLatin1String(".app")) && QFile::moveToTrash(bundle.absolutePath()))
        return {};
    return QStringLiteral("Drag Setu from Applications to the Trash.");
#else
    const QString kind = installKind();
    if (kind == QLatin1String("appimage")) {
        if (QFile::moveToTrash(qEnvironmentVariable("APPIMAGE")))
            return {};
        return QStringLiteral("Delete the Setu AppImage file.");
    }
    if (kind == QLatin1String("flatpak")) {
        if (QProcess::startDetached(QStringLiteral("flatpak-spawn"), {QStringLiteral("--host"), QStringLiteral("flatpak"), QStringLiteral("uninstall"), QStringLiteral("-y"), qEnvironmentVariable("FLATPAK_ID")}))
            return {};
        return QStringLiteral("Remove Setu in your software manager.");
    }
    if (kind == QLatin1String("package"))
        return QStringLiteral("Remove the setu package in your software manager, or with your package manager in a terminal.");
    return QStringLiteral("Delete the folder Setu was unpacked into: %1").arg(QCoreApplication::applicationDirPath());
#endif
}
