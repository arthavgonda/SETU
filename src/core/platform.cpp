#include "platform.h"
#include "profile.h"

#include <QAccessibilityHints>
#include <QClipboard>
#include <QCoreApplication>
#include <QDateTime>
#include <QElapsedTimer>
#include <QDesktopServices>
#include <QEvent>
#include <QFont>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QGuiApplication>
#include <QLocale>
#include <QPalette>
#include <QProcess>
#include <QRegularExpression>
#include <QStyleHints>
#include <QUrl>

#ifdef Q_OS_LINUX
#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusMessage>
#include <QDBusReply>
#include <QDBusVariant>

class PortalWatcher : public QObject
{
    Q_OBJECT
public:
    explicit PortalWatcher(Platform *owner)
        : QObject(owner), m_owner(owner)
    {
        QDBusConnection::sessionBus().connect(QStringLiteral("org.freedesktop.portal.Desktop"), QStringLiteral("/org/freedesktop/portal/desktop"),
                                              QStringLiteral("org.freedesktop.portal.Settings"), QStringLiteral("SettingChanged"), this,
                                              SLOT(onChanged(QString, QString, QDBusVariant)));
    }
public Q_SLOTS:
    void onChanged(const QString &ns, const QString &key, const QDBusVariant &)
    {
        if (ns.contains(QLatin1String("appearance")) || key == QLatin1String("enable-animations") || key == QLatin1String("high-contrast") || key == QLatin1String("text-scaling-factor")) {
            m_owner->readPortal();
            Q_EMIT m_owner->changed();
        }
    }
private:
    Platform *m_owner;
};
#endif

#ifdef Q_OS_WIN
#include <windows.h>
#endif

Platform::Platform(QObject *parent)
    : QObject(parent)
{
    auto *hints = QGuiApplication::styleHints();
    connect(hints, &QStyleHints::colorSchemeChanged, this, &Platform::changed);
    connect(hints->accessibility(), &QAccessibilityHints::contrastPreferenceChanged, this, &Platform::changed);
    qGuiApp->installEventFilter(this);
    connect(qGuiApp, &QGuiApplication::layoutDirectionChanged, this, &Platform::changed);
    readPortal();
    detectTray();
#ifdef Q_OS_LINUX
    new PortalWatcher(this);
#endif
}

#ifdef Q_OS_LINUX
static QVariant portalRead(const QString &ns, const QString &key)
{
    QDBusMessage m = QDBusMessage::createMethodCall(QStringLiteral("org.freedesktop.portal.Desktop"), QStringLiteral("/org/freedesktop/portal/desktop"),
                                                    QStringLiteral("org.freedesktop.portal.Settings"), QStringLiteral("ReadOne"));
    m << ns << key;
    QDBusMessage r = QDBusConnection::sessionBus().call(m, QDBus::Block, 400);
    if (r.type() != QDBusMessage::ReplyMessage || r.arguments().isEmpty())
        return {};
    QVariant v = r.arguments().first();
    while (v.canConvert<QDBusVariant>())
        v = v.value<QDBusVariant>().variant();
    return v;
}
#endif

void Platform::readPortal()
{
#ifdef Q_OS_LINUX
    const QVariant c = portalRead(QStringLiteral("org.freedesktop.appearance"), QStringLiteral("contrast"));
    m_portalContrast = c.isValid() && c.toUInt() == 1;
    const QVariant a = portalRead(QStringLiteral("org.gnome.desktop.interface"), QStringLiteral("enable-animations"));
    m_reduceMotion = a.isValid() && !a.toBool();
    const QVariant hc = portalRead(QStringLiteral("org.gnome.desktop.a11y.interface"), QStringLiteral("high-contrast"));
    if (hc.isValid() && hc.toBool())
        m_portalContrast = true;
    const QVariant scheme = portalRead(QStringLiteral("org.freedesktop.appearance"), QStringLiteral("color-scheme"));
    m_portalScheme = scheme.isValid() ? int(scheme.toUInt()) : 0;
    const QVariant scale = portalRead(QStringLiteral("org.gnome.desktop.interface"), QStringLiteral("text-scaling-factor"));
    m_portalTextScale = scale.isValid() && scale.toDouble() > 0 ? scale.toDouble() : 1.0;
#elif defined(Q_OS_WIN)
    BOOL anim = TRUE;
    if (SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &anim, 0))
        m_reduceMotion = !anim;
    HIGHCONTRASTW hcw{};
    hcw.cbSize = sizeof(hcw);
    if (SystemParametersInfoW(SPI_GETHIGHCONTRAST, sizeof(hcw), &hcw, 0))
        m_portalContrast = (hcw.dwFlags & HCF_HIGHCONTRASTON) != 0;
#endif
}

bool Platform::eventFilter(QObject *watched, QEvent *event)
{
    if (watched == qGuiApp && event->type() == QEvent::ApplicationFontChange)
        Q_EMIT changed();
    return QObject::eventFilter(watched, event);
}

void Platform::detectTray()
{
#ifdef Q_OS_LINUX
    auto *iface = QDBusConnection::sessionBus().interface();
    m_tray = iface && iface->isServiceRegistered(QStringLiteral("org.kde.StatusNotifierWatcher")).value();
#else
    m_tray = true;
#endif
}

bool Platform::darkScheme() const
{
    const Qt::ColorScheme cs = QGuiApplication::styleHints()->colorScheme();
    if (cs != Qt::ColorScheme::Unknown)
        return cs == Qt::ColorScheme::Dark;
    return m_portalScheme == 1;
}

bool Platform::highContrast() const
{
    return m_portalContrast || QGuiApplication::styleHints()->accessibility()->contrastPreference() == Qt::ContrastPreference::HighContrast;
}

qreal Platform::osTextScale() const
{
    const QFont f = QGuiApplication::font();
    qreal base = 10.0;
#if defined(Q_OS_MACOS)
    base = 13.0;
#elif defined(Q_OS_WIN)
    base = 9.0;
#endif
    const qreal pt = f.pointSizeF() > 0 ? f.pointSizeF() : base;
    return qBound(1.0, qMax(pt / base, m_portalTextScale), 2.0);
}

QColor Platform::highlight() const
{
    return QGuiApplication::palette().color(QPalette::Highlight);
}

QColor Platform::highlightedText() const
{
    return QGuiApplication::palette().color(QPalette::HighlightedText);
}

QString Platform::os() const
{
#if defined(Q_OS_MACOS)
    return QStringLiteral("macos");
#elif defined(Q_OS_WIN)
    return QStringLiteral("windows");
#else
    return QStringLiteral("linux");
#endif
}

QString Platform::desktop() const
{
    return qEnvironmentVariable("XDG_CURRENT_DESKTOP");
}

bool Platform::wayland() const
{
    return QGuiApplication::platformName().startsWith(QLatin1String("wayland"));
}

void Platform::loadProfile() const
{
    if (m_profileLoaded)
        return;
    m_profileLoaded = true;
    const QString forced = qEnvironmentVariable("SETU_PROFILE_NAME");
    if (!forced.isEmpty()) {
        m_userName = forced;
        m_userLogin = forced.toLower();
        return;
    }
    const Profile p = profile::read();
    m_userName = p.name;
    m_userLogin = p.login;
    m_userAvatar = p.avatar;
}

QString Platform::userName() const
{
    loadProfile();
    return m_userName;
}

QString Platform::userLogin() const
{
    loadProfile();
    return m_userLogin;
}

QString Platform::userAvatar() const
{
    loadProfile();
    return m_userAvatar;
}

QString Platform::homePath() const
{
    return QDir::homePath();
}

bool Platform::rightToLeft() const
{
    return QGuiApplication::layoutDirection() == Qt::RightToLeft;
}

static QElapsedTimer &processClock()
{
    static QElapsedTimer t = [] {
        QElapsedTimer e;
        e.start();
        return e;
    }();
    return t;
}

static const bool kClockStarted = processClock().isValid();

qint64 Platform::processMs() const
{
    return kClockStarted ? processClock().elapsed() : 0;
}

QVariantMap Platform::overrides() const
{
    QVariantMap out;
    const QString theme = qEnvironmentVariable("SETU_UI_THEME");
    if (theme == QLatin1String("light") || theme == QLatin1String("dark") || theme == QLatin1String("contrast"))
        out.insert(QStringLiteral("theme"), theme);
    bool ok = false;
    const double scale = qEnvironmentVariable("SETU_UI_SCALE").toDouble(&ok);
    if (ok && scale >= 1.0 && scale <= 2.0)
        out.insert(QStringLiteral("scale"), scale);
    if (qEnvironmentVariableIsSet("SETU_UI_TIMING"))
        out.insert(QStringLiteral("timing"), true);
    const QString preview = qEnvironmentVariable("SETU_UI_PREVIEW");
    if (!preview.isEmpty())
        out.insert(QStringLiteral("preview"), preview);
    return out;
}

static QString bundlePath()
{
    QDir d(QCoreApplication::applicationDirPath());
    if (!d.cdUp() || !d.cdUp())
        return {};
    return d.absolutePath();
}

bool Platform::outsideApplications() const
{
    if (qEnvironmentVariable("SETU_UI_PREVIEW") == QLatin1String("move"))
        return true;
    if (os() != QLatin1String("macos"))
        return false;
    const QString b = bundlePath();
    if (QFileInfo(b).fileName() != QStringLiteral(SETU_PRODUCT_NAME ".app"))
        return false;
    return !b.startsWith(QLatin1String("/Applications/")) && !b.startsWith(QDir::homePath() + QLatin1String("/Applications/"));
}

QString Platform::moveToApplications() const
{
    const QString src = bundlePath();
    if (os() != QLatin1String("macos") || !src.endsWith(QLatin1String(".app")))
        return tr("Moving is only needed on macOS.");
    QString dir = QStringLiteral("/Applications");
    if (!QFileInfo(dir).isWritable()) {
        dir = QDir::homePath() + QStringLiteral("/Applications");
        QDir().mkpath(dir);
    }
    const QString dst = dir + QLatin1Char('/') + QFileInfo(src).fileName();
    if (QFileInfo::exists(dst) && !QFile::moveToTrash(dst))
        return tr("An older copy of %1 is in Applications. Move it to the Trash, then try again.").arg(QStringLiteral(SETU_PRODUCT_NAME));
    QProcess ditto;
    ditto.start(QStringLiteral("/usr/bin/ditto"), {src, dst});
    if (!ditto.waitForFinished(180000) || ditto.exitStatus() != QProcess::NormalExit || ditto.exitCode() != 0)
        return tr("Copying failed. %1").arg(QString::fromUtf8(ditto.readAllStandardError()).trimmed());
    const QString volume = src.startsWith(QLatin1String("/Volumes/")) ? src.section(QLatin1Char('/'), 0, 2) : QString();
    QProcess::startDetached(QStringLiteral("/bin/sh"), {QStringLiteral("-c"), QStringLiteral("sleep 2; /usr/bin/open -n \"$0\"; if [ -n \"$1\" ]; then sleep 3; /usr/bin/hdiutil detach \"$1\" -quiet; fi"), dst, volume});
    return {};
}

QString Platform::overlayMode() const
{
#ifdef Q_OS_LINUX
    const QString forced = qEnvironmentVariable("QT_QPA_PLATFORM");
    const QString session = qEnvironmentVariable("XDG_SESSION_TYPE");
    const QString desktop = qEnvironmentVariable("XDG_CURRENT_DESKTOP").toLower();
    if (forced.startsWith(QLatin1String("xcb")))
        return QStringLiteral("x11");
    if (session == QLatin1String("wayland") || forced.startsWith(QLatin1String("wayland"))) {
        if (desktop.contains(QLatin1String("gnome")) && !qEnvironmentVariableIsEmpty("DISPLAY"))
            return QStringLiteral("x11");
#ifdef SETU_HAVE_LAYERSHELL
        if (!desktop.contains(QLatin1String("gnome")))
            return QStringLiteral("layer-shell");
#endif
        return QStringLiteral("notification");
    }
    return QStringLiteral("x11");
#else
    return QStringLiteral("native");
#endif
}

void Platform::openUrl(const QString &url) const
{
    const QUrl u(url);
    if (u.scheme() == QLatin1String("https") || u.scheme() == QLatin1String("http") || u.scheme() == QLatin1String("mailto"))
        QDesktopServices::openUrl(u);
}

void Platform::openPath(const QString &path) const
{
    if (QFileInfo::exists(path))
        QDesktopServices::openUrl(QUrl::fromLocalFile(path));
}

void Platform::copy(const QString &text) const
{
    QGuiApplication::clipboard()->setText(text);
}

QString Platform::localFile(const QUrl &url) const
{
    return url.toLocalFile();
}

QString Platform::formatBytes(qint64 bytes) const
{
    return QLocale().formattedDataSize(bytes, 1, QLocale::DataSizeSIFormat);
}

QString Platform::formatTime(qint64 ms) const
{
    return QLocale().toString(QDateTime::fromMSecsSinceEpoch(ms).time(), QLocale::ShortFormat);
}

QString Platform::formatDateTime(qint64 ms) const
{
    const QDateTime dt = QDateTime::fromMSecsSinceEpoch(ms);
    if (dt.date() == QDate::currentDate())
        return QLocale().toString(dt.time(), QLocale::ShortFormat);
    return QLocale().toString(dt, QLocale::ShortFormat);
}

QString Platform::formatDuration(qint64 seconds) const
{
    if (seconds < 60)
        return tr("%n s", nullptr, int(seconds));
    if (seconds < 3600)
        return tr("%1 min %2 s").arg(seconds / 60).arg(seconds % 60);
    return tr("%1 h %2 min").arg(seconds / 3600).arg((seconds % 3600) / 60);
}

QString Platform::resourceText(const QString &name) const
{
    static const QRegularExpression allowed(QStringLiteral("^licenses/[A-Za-z0-9_.-]+$"));
    if (!allowed.match(name).hasMatch())
        return {};
    QFile f(QStringLiteral(":/") + name);
    if (!f.open(QIODevice::ReadOnly))
        return {};
    return QString::fromUtf8(f.readAll());
}

QString Platform::shortcut(const QString &keys) const
{
    QString k = keys;
#ifdef Q_OS_MACOS
    k.replace(QLatin1String("Ctrl"), QStringLiteral("Cmd"));
#endif
    return k;
}

#ifdef Q_OS_LINUX
#include "platform.moc"
#endif
