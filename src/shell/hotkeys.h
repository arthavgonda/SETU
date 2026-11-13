#pragma once

#include <QAbstractNativeEventFilter>
#include <QHash>
#include <QObject>
#include <QQmlEngine>
#include <QStringList>

class Hotkeys : public QObject, public QAbstractNativeEventFilter
{
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(bool active READ active WRITE setActive NOTIFY activeChanged)
    Q_PROPERTY(QStringList keys READ keys WRITE setKeys NOTIFY keysChanged)
    Q_PROPERTY(bool available READ available CONSTANT)

public:
    explicit Hotkeys(QObject *parent = nullptr);
    ~Hotkeys() override;

    bool active() const { return m_active; }
    void setActive(bool a);
    QStringList keys() const { return m_keys; }
    void setKeys(const QStringList &k);
    bool available() const;

    bool nativeEventFilter(const QByteArray &eventType, void *message, qintptr *result) override;

signals:
    void activeChanged();
    void keysChanged();
    void pressed(const QString &key);

private:
    void grab();
    void release();
    bool m_active = false;
    bool m_grabbed = false;
    QStringList m_keys;
    QHash<quint32, QString> m_codes;
};
