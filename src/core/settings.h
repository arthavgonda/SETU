#pragma once

#include <QObject>
#include <QQmlEngine>
#include <QSet>
#include <QVariantMap>

class SettingsStore : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(Settings)
    QML_SINGLETON
    Q_PROPERTY(QVariantMap values READ values NOTIFY valuesChanged)
    Q_PROPERTY(bool loaded READ loaded NOTIFY valuesChanged)
    Q_PROPERTY(qint64 savedAt READ savedAt NOTIFY savedAtChanged)

public:
    explicit SettingsStore(QObject *parent = nullptr);
    static SettingsStore *instance();
    static SettingsStore *create(QQmlEngine *, QJSEngine *);

    QVariantMap values() const { return m_values; }
    bool loaded() const { return m_loaded; }
    qint64 savedAt() const { return m_savedAt; }

    Q_INVOKABLE QVariant get(const QString &key, const QVariant &fallback = {}) const;
    Q_INVOKABLE void set(const QString &key, const QVariant &value);
    Q_INVOKABLE void patch(const QVariantMap &values);
    Q_INVOKABLE bool pending(const QString &key) const { return m_pending.contains(key); }

signals:
    void valuesChanged();
    void savedAtChanged();
    void saved(const QString &key);
    void rejected(const QString &key, const QString &message);

private:
    void load(const QVariantMap &all);
    void merge(const QVariantMap &changes);
    QVariantMap m_values;
    QSet<QString> m_pending;
    bool m_loaded = false;
    qint64 m_savedAt = 0;
};
