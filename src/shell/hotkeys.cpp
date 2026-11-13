#include "hotkeys.h"

#include <QGuiApplication>

#ifdef SETU_HAVE_XCB
#include <xcb/xcb.h>
#endif
#ifdef Q_OS_WIN
#include <windows.h>
#endif

namespace {
quint32 keysymFor(const QString &key)
{
    if (key == QLatin1String("Escape"))
        return 0xff1b;
    if (key == QLatin1String("Return"))
        return 0xff0d;
    if (key.size() == 1 && key.at(0).isDigit())
        return 0x30 + quint32(key.at(0).digitValue());
    return 0;
}

#ifdef Q_OS_WIN
UINT vkFor(const QString &key)
{
    if (key == QLatin1String("Escape"))
        return VK_ESCAPE;
    if (key == QLatin1String("Return"))
        return VK_RETURN;
    if (key.size() == 1 && key.at(0).isDigit())
        return UINT('0' + key.at(0).digitValue());
    return 0;
}
#endif
}

Hotkeys::Hotkeys(QObject *parent)
    : QObject(parent)
{
    qGuiApp->installNativeEventFilter(this);
}

Hotkeys::~Hotkeys()
{
    release();
    qGuiApp->removeNativeEventFilter(this);
}

bool Hotkeys::available() const
{
#ifdef SETU_HAVE_XCB
    if (QGuiApplication::platformName() == QLatin1String("xcb"))
        return true;
#endif
#ifdef Q_OS_WIN
    return true;
#else
    return false;
#endif
}

void Hotkeys::setActive(bool a)
{
    if (a == m_active)
        return;
    m_active = a;
    emit activeChanged();
    if (a)
        grab();
    else
        release();
}

void Hotkeys::setKeys(const QStringList &k)
{
    if (k == m_keys)
        return;
    const bool was = m_grabbed;
    release();
    m_keys = k;
    emit keysChanged();
    if (was || m_active)
        grab();
}

void Hotkeys::grab()
{
    if (m_grabbed || m_keys.isEmpty())
        return;
#ifdef SETU_HAVE_XCB
    if (QGuiApplication::platformName() == QLatin1String("xcb")) {
        auto *x11 = qGuiApp->nativeInterface<QNativeInterface::QX11Application>();
        if (!x11)
            return;
        xcb_connection_t *c = x11->connection();
        const xcb_setup_t *setup = xcb_get_setup(c);
        const xcb_window_t root = xcb_setup_roots_iterator(setup).data->root;
        const xcb_keycode_t first = setup->min_keycode;
        const uint8_t count = setup->max_keycode - setup->min_keycode + 1;
        auto *map = xcb_get_keyboard_mapping_reply(c, xcb_get_keyboard_mapping(c, first, count), nullptr);
        if (!map)
            return;
        const int per = map->keysyms_per_keycode;
        const xcb_keysym_t *syms = xcb_get_keyboard_mapping_keysyms(map);
        m_codes.clear();
        for (const QString &k : std::as_const(m_keys)) {
            const quint32 want = keysymFor(k);
            for (int i = 0; i < count; ++i) {
                if (syms[i * per] == want) {
                    const xcb_keycode_t code = xcb_keycode_t(first + i);
                    m_codes.insert(code, k);
                    for (uint16_t mods : {uint16_t(0), uint16_t(XCB_MOD_MASK_2), uint16_t(XCB_MOD_MASK_LOCK), uint16_t(XCB_MOD_MASK_2 | XCB_MOD_MASK_LOCK)})
                        xcb_grab_key(c, 0, root, mods, code, XCB_GRAB_MODE_ASYNC, XCB_GRAB_MODE_ASYNC);
                    break;
                }
            }
        }
        free(map);
        xcb_flush(c);
        m_grabbed = true;
    }
#endif
#ifdef Q_OS_WIN
    m_codes.clear();
    int id = 1;
    for (const QString &k : std::as_const(m_keys)) {
        if (RegisterHotKey(nullptr, id, MOD_NOREPEAT, vkFor(k)))
            m_codes.insert(quint32(id), k);
        ++id;
    }
    m_grabbed = true;
#endif
}

void Hotkeys::release()
{
    if (!m_grabbed)
        return;
#ifdef SETU_HAVE_XCB
    if (QGuiApplication::platformName() == QLatin1String("xcb")) {
        if (auto *x11 = qGuiApp->nativeInterface<QNativeInterface::QX11Application>()) {
            xcb_connection_t *c = x11->connection();
            const xcb_window_t root = xcb_setup_roots_iterator(xcb_get_setup(c)).data->root;
            for (auto it = m_codes.cbegin(); it != m_codes.cend(); ++it)
                xcb_ungrab_key(c, xcb_keycode_t(it.key()), root, XCB_MOD_MASK_ANY);
            xcb_flush(c);
        }
    }
#endif
#ifdef Q_OS_WIN
    for (auto it = m_codes.cbegin(); it != m_codes.cend(); ++it)
        UnregisterHotKey(nullptr, int(it.key()));
#endif
    m_codes.clear();
    m_grabbed = false;
}

bool Hotkeys::nativeEventFilter(const QByteArray &eventType, void *message, qintptr *result)
{
    Q_UNUSED(result)
    if (!m_grabbed)
        return false;
#ifdef SETU_HAVE_XCB
    if (eventType == "xcb_generic_event_t") {
        auto *ev = static_cast<xcb_generic_event_t *>(message);
        if ((ev->response_type & 0x7f) == XCB_KEY_PRESS) {
            auto *k = reinterpret_cast<xcb_key_press_event_t *>(ev);
            const auto it = m_codes.constFind(k->detail);
            if (it != m_codes.cend()) {
                emit pressed(it.value());
                return true;
            }
        }
    }
#endif
#ifdef Q_OS_WIN
    if (eventType == "windows_generic_MSG") {
        auto *msg = static_cast<MSG *>(message);
        if (msg->message == WM_HOTKEY) {
            const auto it = m_codes.constFind(quint32(msg->wParam));
            if (it != m_codes.cend()) {
                emit pressed(it.value());
                return true;
            }
        }
    }
#endif
    Q_UNUSED(eventType)
    Q_UNUSED(message)
    return false;
}
