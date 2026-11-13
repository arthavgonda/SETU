#include "iconprovider.h"

#include <QAbstractFileIconProvider>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QIcon>
#include <QImageReader>
#include <QPainter>
#include <QPainterPath>
#include <QSvgRenderer>
#include <QUrlQuery>

namespace {
QByteArray svgFor(const QString &name, const QString &color, qreal stroke)
{
    QFile f(QStringLiteral(":/icons/%1.svg").arg(name));
    if (!f.open(QIODevice::ReadOnly))
        return {};
    QByteArray b = f.readAll();
    b.replace("currentColor", color.toLatin1());
    b.replace("stroke-width=\"2\"", QByteArray("stroke-width=\"") + QByteArray::number(stroke) + "\"");
    return b;
}
}

IconProvider::IconProvider()
    : QQuickImageProvider(QQuickImageProvider::Image), m_cache(512)
{
}

QImage IconProvider::requestImage(const QString &id, QSize *size, const QSize &requested)
{
    const int qpos = id.indexOf(QLatin1Char('?'));
    const QString name = qpos < 0 ? id : id.left(qpos);
    const QUrlQuery q(qpos < 0 ? QString() : id.mid(qpos + 1));
    const QString color = q.hasQueryItem(QStringLiteral("color")) ? QStringLiteral("#") + q.queryItemValue(QStringLiteral("color")) : QStringLiteral("#000000");
    const QSize out = requested.isValid() && !requested.isEmpty() ? requested : QSize(20, 20);
    const QString key = name + color + QString::number(out.width()) + QLatin1Char('x') + QString::number(out.height());
    {
        QMutexLocker l(&m_lock);
        if (QImage *hit = m_cache.object(key)) {
            if (size)
                *size = hit->size();
            return *hit;
        }
    }
    QImage img(out, QImage::Format_ARGB32_Premultiplied);
    img.fill(Qt::transparent);
    QSvgRenderer r(svgFor(name, color, 1.8));
    if (r.isValid()) {
        QPainter p(&img);
        p.setRenderHint(QPainter::Antialiasing);
        r.render(&p);
    }
    if (size)
        *size = img.size();
    QMutexLocker l(&m_lock);
    m_cache.insert(key, new QImage(img));
    return img;
}

TrayIconProvider::TrayIconProvider()
    : QQuickImageProvider(QQuickImageProvider::Image)
{
}

QImage TrayIconProvider::requestImage(const QString &id, QSize *size, const QSize &requested)
{
    const int qpos = id.indexOf(QLatin1Char('?'));
    const QString state = qpos < 0 ? id : id.left(qpos);
    const QUrlQuery q(qpos < 0 ? QString() : id.mid(qpos + 1));
    const QColor ink(q.hasQueryItem(QStringLiteral("ink")) ? QStringLiteral("#") + q.queryItemValue(QStringLiteral("ink")) : QStringLiteral("#ffffff"));
    const QColor accent(q.hasQueryItem(QStringLiteral("accent")) ? QStringLiteral("#") + q.queryItemValue(QStringLiteral("accent")) : QStringLiteral("#f0743a"));
    const int s = requested.isValid() && requested.width() > 0 ? requested.width() : 64;
    QImage img(s, s, QImage::Format_ARGB32_Premultiplied);
    img.fill(Qt::transparent);
    QPainter p(&img);
    p.setRenderHint(QPainter::Antialiasing);
    const QString glyph = state == QLatin1String("muted") ? QStringLiteral("mic-off") : QStringLiteral("mic");
    QSvgRenderer r(svgFor(glyph, ink.name(), 2.2));
    r.render(&p, QRectF(0, 0, s * 0.78, s * 0.78));
    const QRectF mark(s * 0.58, s * 0.58, s * 0.40, s * 0.40);
    QPen pen(ink, s * 0.08);
    p.setPen(Qt::NoPen);
    if (state == QLatin1String("idle")) {
        p.setBrush(ink);
        p.drawEllipse(mark.adjusted(s * 0.06, s * 0.06, -s * 0.06, -s * 0.06));
    } else if (state == QLatin1String("active")) {
        p.setBrush(accent);
        p.drawEllipse(mark);
    } else if (state == QLatin1String("working")) {
        p.setBrush(Qt::NoBrush);
        pen.setColor(accent);
        p.setPen(pen);
        p.drawEllipse(mark.adjusted(s * 0.05, s * 0.05, -s * 0.05, -s * 0.05));
    } else if (state == QLatin1String("starting")) {
        p.setBrush(Qt::NoBrush);
        p.setPen(pen);
        p.drawEllipse(mark.adjusted(s * 0.05, s * 0.05, -s * 0.05, -s * 0.05));
    } else if (state == QLatin1String("attention")) {
        QPainterPath tri;
        tri.moveTo(mark.center().x(), mark.top());
        tri.lineTo(mark.right(), mark.bottom());
        tri.lineTo(mark.left(), mark.bottom());
        tri.closeSubpath();
        p.setBrush(accent);
        p.drawPath(tri);
    } else if (state == QLatin1String("stopped")) {
        p.setPen(pen);
        p.drawLine(mark.topLeft(), mark.bottomRight());
        p.drawLine(mark.topRight(), mark.bottomLeft());
    }
    p.end();
    if (size)
        *size = img.size();
    return img;
}

namespace {
QImage themeFile(const QString &name, const QSize &out)
{
    if (name.contains(QLatin1Char('/')) || name.contains(QLatin1String("..")))
        return {};
    QStringList roots;
    const QString dataHome = qEnvironmentVariable("XDG_DATA_HOME", QDir::homePath() + QStringLiteral("/.local/share"));
    roots << dataHome + QStringLiteral("/icons/hicolor") << dataHome + QStringLiteral("/flatpak/exports/share/icons/hicolor");
    const QStringList dirs = qEnvironmentVariable("XDG_DATA_DIRS", QStringLiteral("/usr/local/share:/usr/share")).split(QLatin1Char(':'), Qt::SkipEmptyParts);
    for (const QString &d : dirs)
        roots << d + QStringLiteral("/icons/hicolor");
    roots << QStringLiteral("/var/lib/flatpak/exports/share/icons/hicolor") << QStringLiteral("/var/lib/snapd/desktop/icons");
    const QStringList sizes{QStringLiteral("scalable"), QStringLiteral("512x512"), QStringLiteral("256x256"), QStringLiteral("192x192"), QStringLiteral("128x128"), QStringLiteral("96x96"), QStringLiteral("64x64"), QStringLiteral("48x48")};
    for (const QString &root : roots) {
        for (const QString &size : sizes) {
            for (const char *ext : {".svg", ".png"}) {
                const QString path = root + QLatin1Char('/') + size + QStringLiteral("/apps/") + name + QLatin1String(ext);
                if (!QFileInfo::exists(path))
                    continue;
                if (path.endsWith(QLatin1String(".svg"))) {
                    QSvgRenderer r(path);
                    if (!r.isValid())
                        continue;
                    QImage img(out, QImage::Format_ARGB32_Premultiplied);
                    img.fill(Qt::transparent);
                    QPainter p(&img);
                    p.setRenderHint(QPainter::Antialiasing);
                    r.render(&p);
                    return img;
                }
                QImage img(path);
                if (!img.isNull())
                    return img;
            }
        }
    }
    for (const char *ext : {".svg", ".png", ".xpm"}) {
        const QString path = QStringLiteral("/usr/share/pixmaps/") + name + QLatin1String(ext);
        QImage img(path);
        if (!img.isNull())
            return img;
    }
    return {};
}
}

AppIconProvider::AppIconProvider()
    : QQuickImageProvider(QQuickImageProvider::Image), m_cache(64)
{
}

QImage AppIconProvider::requestImage(const QString &id, QSize *size, const QSize &requested)
{
    const QString source = QUrl::fromPercentEncoding(id.toUtf8());
    const QSize out = requested.isValid() && !requested.isEmpty() ? requested : QSize(64, 64);
    const QString key = source + QLatin1Char('@') + QString::number(out.width());
    {
        QMutexLocker l(&m_lock);
        if (QImage *hit = m_cache.object(key)) {
            if (size)
                *size = hit->size();
            return *hit;
        }
    }
    QImage img;
    const QFileInfo info(source);
    const QString suffix = info.suffix().toLower();
    if (info.isAbsolute() && info.exists()) {
        if (suffix == QLatin1String("svg") || suffix == QLatin1String("svgz")) {
            QSvgRenderer r(source);
            if (r.isValid()) {
                img = QImage(out, QImage::Format_ARGB32_Premultiplied);
                img.fill(Qt::transparent);
                QPainter p(&img);
                p.setRenderHint(QPainter::Antialiasing);
                r.render(&p);
            }
        } else if (QImageReader::supportedImageFormats().contains(suffix.toLatin1()) && suffix != QLatin1String("ico")) {
            img.load(source);
        } else {
            QFileInfo target = info;
#ifdef Q_OS_WIN
            if (info.isShortcut() && QFileInfo::exists(info.symLinkTarget()))
                target = QFileInfo(info.symLinkTarget());
#endif
            QAbstractFileIconProvider provider;
            const QIcon icon = provider.icon(target);
            if (!icon.isNull())
                img = icon.pixmap(out).toImage();
        }
    } else if (!source.isEmpty()) {
        const QIcon icon = QIcon::fromTheme(source);
        if (!icon.isNull())
            img = icon.pixmap(out).toImage();
        if (img.isNull())
            img = themeFile(source, out);
    }
    if (img.isNull()) {
        if (size)
            *size = QSize();
        return {};
    }
    if (img.size() != out)
        img = img.scaled(out, Qt::KeepAspectRatio, Qt::SmoothTransformation);
    if (size)
        *size = img.size();
    QMutexLocker l(&m_lock);
    m_cache.insert(key, new QImage(img));
    return img;
}
