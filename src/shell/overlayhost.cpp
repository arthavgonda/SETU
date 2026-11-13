#include "overlayhost.h"

#include <QCursor>
#include <QDir>
#include <QGuiApplication>
#include <QFileInfo>
#include <QImage>
#include <QRegion>
#include <QScreen>

#ifdef SETU_HAVE_LAYERSHELL
#include <LayerShellQt/window.h>
#endif
#ifdef SETU_HAVE_XCB
#include <xcb/shape.h>
#include <xcb/xcb.h>
#endif

#ifdef Q_OS_MACOS
void setuRaiseOverMenuBar(QWindow *window);
#endif

OverlayHost::OverlayHost(QObject *parent)
    : QObject(parent)
{
    connect(qGuiApp, &QGuiApplication::primaryScreenChanged, this, [this](QScreen *s) {
        watch(s);
        emit screensChanged();
    });
    watch(QGuiApplication::primaryScreen());
}

void OverlayHost::watch(QScreen *screen)
{
    if (!screen)
        return;
    connect(screen, &QScreen::geometryChanged, this, &OverlayHost::screensChanged, Qt::UniqueConnection);
    connect(screen, &QScreen::availableGeometryChanged, this, &OverlayHost::screensChanged, Qt::UniqueConnection);
}

QString OverlayHost::shots() const
{
    return qEnvironmentVariable("SETU_SHELL_SHOTS");
}

QRect OverlayHost::placeTop(QQuickWindow *window, int width, int height)
{
    if (!window)
        return {};
    QScreen *s = QGuiApplication::primaryScreen();
    const QRect full = s ? s->geometry() : QRect(0, 0, 1280, 720);
    const QRect avail = s ? s->availableGeometry() : full;
#if defined(Q_OS_MACOS) || defined(Q_OS_WIN)
    const int top = full.top();
#else
    const int top = avail.top();
#endif
    const int w = qMin(width, full.width());
    const int x = full.left() + (full.width() - w) / 2;
    if (s && window->screen() != s)
        window->setScreen(s);
#ifdef SETU_HAVE_LAYERSHELL
    if (mode() == QLatin1String("layer-shell")) {
        if (auto *ls = LayerShellQt::Window::get(window)) {
            ls->setAnchors(LayerShellQt::Window::Anchors(LayerShellQt::Window::AnchorTop));
            ls->setMargins(QMargins(0, 0, 0, 0));
        }
        window->resize(w, height);
        return QRect(x, top, w, height);
    }
#endif
    window->setGeometry(x, top, w, height);
    return QRect(x, top, w, height);
}

void OverlayHost::setShape(QQuickWindow *window, const QVariantList &rects)
{
    if (!window)
        return;
#if defined(Q_OS_WIN) || defined(Q_OS_MACOS)
    QRegion region;
    for (const auto &v : rects)
        region += v.toRect();
    window->setMask(region.isEmpty() ? QRegion(0, 0, 1, 1) : region);
#else
    setInputRegion(window, rects);
#endif
}

void OverlayHost::raiseNotch(QQuickWindow *window)
{
    if (!window)
        return;
#ifdef Q_OS_MACOS
    setuRaiseOverMenuBar(window);
#endif
    window->raise();
}

bool OverlayHost::capture(QQuickWindow *window, const QString &path)
{
    if (!window || path.isEmpty())
        return false;
    QDir().mkpath(QFileInfo(path).absolutePath());
    return window->grabWindow().save(path);
}

QString OverlayHost::mode() const
{
    const QString p = QGuiApplication::platformName();
    if (p.startsWith(QLatin1String("wayland"))) {
#ifdef SETU_HAVE_LAYERSHELL
        const QString d = qEnvironmentVariable("XDG_CURRENT_DESKTOP").toLower();
        if (!d.contains(QLatin1String("gnome")))
            return QStringLiteral("layer-shell");
#endif
        return QStringLiteral("notification");
    }
    if (p == QLatin1String("xcb"))
        return QStringLiteral("x11");
    return p;
}

bool OverlayHost::fallback() const
{
    return mode() == QLatin1String("notification");
}

void OverlayHost::prepare(QQuickWindow *window, bool clickThrough)
{
    if (!window)
        return;
    Qt::WindowFlags f = Qt::FramelessWindowHint | Qt::WindowStaysOnTopHint | Qt::WindowDoesNotAcceptFocus | Qt::Tool | Qt::NoDropShadowWindowHint;
    if (mode() == QLatin1String("x11"))
        f |= Qt::BypassWindowManagerHint | Qt::X11BypassWindowManagerHint;
    if (clickThrough)
        f |= Qt::WindowTransparentForInput;
    window->setFlags(f);
    window->setColor(Qt::transparent);
#ifdef SETU_HAVE_LAYERSHELL
    if (mode() == QLatin1String("layer-shell")) {
        if (auto *ls = LayerShellQt::Window::get(window)) {
            ls->setLayer(LayerShellQt::Window::LayerOverlay);
            ls->setKeyboardInteractivity(LayerShellQt::Window::KeyboardInteractivityNone);
            ls->setExclusiveZone(-1);
            ls->setScope(QStringLiteral("setu-overlay"));
            ls->setAnchors(LayerShellQt::Window::Anchors(LayerShellQt::Window::AnchorBottom));
        }
    }
#endif
}

QRect OverlayHost::virtualGeometry() const
{
    QScreen *s = QGuiApplication::screenAt(QCursor::pos());
    if (!s)
        s = QGuiApplication::primaryScreen();
    return s ? s->geometry() : QRect(0, 0, 1280, 720);
}

QRect OverlayHost::place(QQuickWindow *window, int width, int height, int bottomMargin, int align, int sideMargin)
{
    if (!window)
        return {};
    QScreen *s = QGuiApplication::screenAt(QCursor::pos());
    if (!s)
        s = QGuiApplication::primaryScreen();
    const QRect area = s ? s->availableGeometry() : QRect(0, 0, 1280, 720);
    const int w = qMin(width, area.width() - 32);
    const int side = sideMargin > 0 ? sideMargin : bottomMargin;
    const int x = align > 0 ? area.right() - w - side + 1 : align < 0 ? area.left() + side : area.left() + (area.width() - w) / 2;
    const int y = area.bottom() - height - bottomMargin + 1;
    if (s && window->screen() != s)
        window->setScreen(s);
#ifdef SETU_HAVE_LAYERSHELL
    if (mode() == QLatin1String("layer-shell")) {
        if (auto *ls = LayerShellQt::Window::get(window)) {
            ls->setAnchors(align > 0 ? LayerShellQt::Window::Anchors(LayerShellQt::Window::AnchorBottom | LayerShellQt::Window::AnchorRight)
                           : align < 0 ? LayerShellQt::Window::Anchors(LayerShellQt::Window::AnchorBottom | LayerShellQt::Window::AnchorLeft)
                                       : LayerShellQt::Window::Anchors(LayerShellQt::Window::AnchorBottom));
            ls->setMargins(QMargins(align < 0 ? side : 0, 0, align > 0 ? side : 0, bottomMargin));
        }
        window->resize(w, height);
        return QRect(x, y, w, height);
    }
#endif
    window->setGeometry(x, y, w, height);
    return QRect(x, y, w, height);
}

void OverlayHost::setInputRegion(QQuickWindow *window, const QVariantList &rects)
{
    if (!window)
        return;
    QRegion region;
    for (const auto &v : rects)
        region += v.toRect();
#ifdef SETU_HAVE_XCB
    if (mode() == QLatin1String("x11")) {
        auto *x11 = qGuiApp->nativeInterface<QNativeInterface::QX11Application>();
        if (x11 && window->handle()) {
            xcb_connection_t *c = x11->connection();
            const qreal dpr = window->devicePixelRatio();
            QList<xcb_rectangle_t> list;
            for (const QRect &r : region)
                list.append(xcb_rectangle_t{int16_t(r.x() * dpr), int16_t(r.y() * dpr), uint16_t(r.width() * dpr), uint16_t(r.height() * dpr)});
            xcb_shape_rectangles(c, XCB_SHAPE_SO_SET, XCB_SHAPE_SK_INPUT, XCB_CLIP_ORDERING_UNSORTED, xcb_window_t(window->winId()), 0, 0, uint32_t(list.size()),
                                 list.constData());
            xcb_flush(c);
            return;
        }
    }
#endif
    if (QGuiApplication::platformName().startsWith(QLatin1String("wayland")))
        window->setMask(region.isEmpty() ? QRegion(-1, -1, 1, 1) : region);
}
