#include "profile.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QImage>
#include <QPainter>
#include <QPainterPath>
#include <QProcess>
#include <QSettings>
#include <QStandardPaths>
#include <QUrl>

#ifdef Q_OS_LINUX
#include <QDBusConnection>
#include <QDBusInterface>
#include <QDBusObjectPath>
#include <QDBusReply>
#include <pwd.h>
#include <unistd.h>
#endif

#ifdef Q_OS_WIN
#include <windows.h>
#include <lm.h>
#include <sddl.h>
#endif

namespace {

QString capitalized(const QString &s)
{
    if (s.isEmpty())
        return s;
    return s.left(1).toUpper() + s.mid(1);
}

QString cacheFile(const QString &name)
{
    const QString dir = QStandardPaths::writableLocation(QStandardPaths::CacheLocation);
    QDir().mkpath(dir);
    return dir + QLatin1Char('/') + name;
}

QString roundAvatar(const QImage &source)
{
    if (source.isNull())
        return {};
    const int side = 160;
    const QImage square = source.scaled(side, side, Qt::KeepAspectRatioByExpanding, Qt::SmoothTransformation);
    QImage out(side, side, QImage::Format_ARGB32_Premultiplied);
    out.fill(Qt::transparent);
    QPainter p(&out);
    p.setRenderHint(QPainter::Antialiasing);
    p.setRenderHint(QPainter::SmoothPixmapTransform);
    QPainterPath clip;
    clip.addEllipse(QRectF(0, 0, side, side));
    p.setClipPath(clip);
    p.drawImage(QPoint((side - square.width()) / 2, (side - square.height()) / 2), square);
    p.end();
    const QString path = cacheFile(QStringLiteral("profile-avatar.png"));
    if (!out.save(path, "PNG"))
        return {};
    return QUrl::fromLocalFile(path).toString() + QStringLiteral("?v=") + QString::number(QFileInfo(path).lastModified().toMSecsSinceEpoch());
}

QImage embeddedImage(const QByteArray &data)
{
    QImage best;
    const QByteArray png("\x89PNG\r\n\x1a\n", 8);
    const QByteArray jpg("\xff\xd8\xff", 3);
    for (const QByteArray &sig : {png, jpg}) {
        int at = data.indexOf(sig);
        while (at >= 0) {
            QImage img;
            img.loadFromData(data.mid(at));
            if (!img.isNull() && img.width() > best.width())
                best = img;
            at = data.indexOf(sig, at + 1);
        }
    }
    return best;
}

#ifdef Q_OS_MACOS
QString dscl(const QString &attr)
{
    QProcess p;
    p.start(QStringLiteral("/usr/bin/dscl"), {QStringLiteral("."), QStringLiteral("-read"), QStringLiteral("/Users/") + qEnvironmentVariable("USER"), attr});
    if (!p.waitForFinished(2000))
        return {};
    return QString::fromUtf8(p.readAllStandardOutput());
}
#endif

}

Profile profile::read()
{
    Profile pr;
#ifdef Q_OS_LINUX
    if (const passwd *pw = getpwuid(getuid())) {
        pr.login = QString::fromLocal8Bit(pw->pw_name);
        pr.name = QString::fromLocal8Bit(pw->pw_gecos).section(QLatin1Char(','), 0, 0).trimmed();
    }
    QString icon;
    QDBusInterface accounts(QStringLiteral("org.freedesktop.Accounts"), QStringLiteral("/org/freedesktop/Accounts"), QStringLiteral("org.freedesktop.Accounts"), QDBusConnection::systemBus());
    if (accounts.isValid()) {
        QDBusReply<QDBusObjectPath> user = accounts.call(QStringLiteral("FindUserById"), qint64(getuid()));
        if (user.isValid()) {
            QDBusInterface u(QStringLiteral("org.freedesktop.Accounts"), user.value().path(), QStringLiteral("org.freedesktop.Accounts.User"), QDBusConnection::systemBus());
            if (u.isValid()) {
                const QString real = u.property("RealName").toString().trimmed();
                if (!real.isEmpty())
                    pr.name = real;
                icon = u.property("IconFile").toString();
            }
        }
    }
    QStringList candidates{icon, QDir::homePath() + QStringLiteral("/.face"), QDir::homePath() + QStringLiteral("/.face.icon")};
    for (const QString &c : candidates) {
        if (!c.isEmpty() && QFileInfo(c).isFile() && QFileInfo(c).size() > 0) {
            QFile f(c);
            if (f.open(QIODevice::ReadOnly)) {
                QImage img;
                img.loadFromData(f.readAll());
                pr.avatar = roundAvatar(img);
                if (!pr.avatar.isEmpty())
                    break;
            }
        }
    }
#elif defined(Q_OS_WIN)
    wchar_t login[256];
    DWORD len = 256;
    if (GetUserNameW(login, &len))
        pr.login = QString::fromWCharArray(login);
    USER_INFO_10 *info = nullptr;
    if (NetUserGetInfo(nullptr, reinterpret_cast<LPCWSTR>(pr.login.utf16()), 10, reinterpret_cast<LPBYTE *>(&info)) == NERR_Success && info) {
        pr.name = QString::fromWCharArray(info->usri10_full_name).trimmed();
        NetApiBufferFree(info);
    }
    HANDLE token = nullptr;
    QString sid;
    if (OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) {
        DWORD size = 0;
        GetTokenInformation(token, TokenUser, nullptr, 0, &size);
        QByteArray buf(int(size), 0);
        if (size && GetTokenInformation(token, TokenUser, buf.data(), size, &size)) {
            LPWSTR text = nullptr;
            if (ConvertSidToStringSidW(reinterpret_cast<TOKEN_USER *>(buf.data())->User.Sid, &text)) {
                sid = QString::fromWCharArray(text);
                LocalFree(text);
            }
        }
        CloseHandle(token);
    }
    QImage img;
    if (!sid.isEmpty()) {
        QSettings reg(QStringLiteral("HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\AccountPicture\\Users\\") + sid, QSettings::NativeFormat);
        for (const char *k : {"Image448", "Image240", "Image192", "Image96"}) {
            const QString path = reg.value(QLatin1String(k)).toString();
            if (!path.isEmpty() && img.load(path))
                break;
        }
    }
    if (img.isNull()) {
        QDir dir(qEnvironmentVariable("APPDATA") + QStringLiteral("/Microsoft/Windows/AccountPictures"));
        const auto files = dir.entryInfoList({QStringLiteral("*.accountpicture-ms")}, QDir::Files, QDir::Time);
        if (!files.isEmpty()) {
            QFile f(files.first().absoluteFilePath());
            if (f.open(QIODevice::ReadOnly))
                img = embeddedImage(f.readAll());
        }
    }
    pr.avatar = roundAvatar(img);
#elif defined(Q_OS_MACOS)
    pr.login = qEnvironmentVariable("USER");
    const QString real = dscl(QStringLiteral("RealName"));
    const QStringList lines = real.split(QLatin1Char('\n'), Qt::SkipEmptyParts);
    if (lines.size() >= 2)
        pr.name = lines.at(1).trimmed();
    else if (!lines.isEmpty())
        pr.name = lines.first().section(QLatin1Char(':'), 1).trimmed();
    const QString photo = dscl(QStringLiteral("JPEGPhoto"));
    QByteArray bytes;
    for (const QString &line : photo.split(QLatin1Char('\n')).mid(1))
        bytes += QByteArray::fromHex(line.simplified().remove(QLatin1Char(' ')).toLatin1());
    QImage img;
    img.loadFromData(bytes);
    if (img.isNull()) {
        const QString pic = dscl(QStringLiteral("Picture")).section(QLatin1Char('\n'), 1).trimmed();
        if (!pic.isEmpty())
            img.load(pic);
    }
    pr.avatar = roundAvatar(img);
#endif
    if (pr.login.isEmpty())
        pr.login = qEnvironmentVariable("USER", qEnvironmentVariable("USERNAME"));
    if (pr.name.isEmpty())
        pr.name = capitalized(pr.login);
    return pr;
}
