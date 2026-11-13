#include "settings.h"
#include "daemonclient.h"

#include <QDateTime>

namespace {
SettingsStore *g_settings = nullptr;
}

SettingsStore::SettingsStore(QObject *parent)
    : QObject(parent)
{
    g_settings = this;
    if (auto *d = DaemonClient::instance()) {
        connect(d, &DaemonClient::settingsLoaded, this, &SettingsStore::load);
        connect(d, &DaemonClient::settingsChanged, this, &SettingsStore::merge);
    }
}

SettingsStore *SettingsStore::instance()
{
    return g_settings;
}

SettingsStore *SettingsStore::create(QQmlEngine *, QJSEngine *)
{
    QJSEngine::setObjectOwnership(g_settings, QJSEngine::CppOwnership);
    return g_settings;
}

QVariant SettingsStore::get(const QString &key, const QVariant &fallback) const
{
    return m_values.value(key, fallback);
}

void SettingsStore::load(const QVariantMap &all)
{
    m_values = all;
    m_pending.clear();
    m_loaded = true;
    emit valuesChanged();
}

void SettingsStore::merge(const QVariantMap &changes)
{
    bool any = false;
    for (auto it = changes.cbegin(); it != changes.cend(); ++it) {
        if (m_pending.contains(it.key()))
            continue;
        if (m_values.value(it.key()) != it.value()) {
            m_values.insert(it.key(), it.value());
            any = true;
        }
    }
    if (any)
        emit valuesChanged();
}

void SettingsStore::set(const QString &key, const QVariant &value)
{
    patch({{key, value}});
}

void SettingsStore::patch(const QVariantMap &values)
{
    auto *d = DaemonClient::instance();
    QVariantMap before;
    for (auto it = values.cbegin(); it != values.cend(); ++it) {
        before.insert(it.key(), m_values.value(it.key()));
        m_values.insert(it.key(), it.value());
        m_pending.insert(it.key());
    }
    emit valuesChanged();
    if (!d)
        return;
    d->request(QStringLiteral("settings.set"), {{QStringLiteral("patch"), values}}, [this, before](bool ok, const QVariant &result, const QVariantMap &error) {
        for (auto it = before.cbegin(); it != before.cend(); ++it)
            m_pending.remove(it.key());
        if (!ok) {
            for (auto it = before.cbegin(); it != before.cend(); ++it)
                m_values.insert(it.key(), it.value());
            emit valuesChanged();
            emit rejected(before.firstKey(), error.value(QStringLiteral("message")).toString());
            return;
        }
        const QVariantMap applied = result.toMap().value(QStringLiteral("settings")).toMap();
        for (auto it = applied.cbegin(); it != applied.cend(); ++it)
            m_values.insert(it.key(), it.value());
        emit valuesChanged();
        m_savedAt = QDateTime::currentMSecsSinceEpoch();
        emit savedAtChanged();
        for (auto it = before.cbegin(); it != before.cend(); ++it)
            emit saved(it.key());
    });
}
