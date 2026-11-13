#pragma once

#include <QObject>
#include <QQmlEngine>
#include <QQmlParserStatus>
#include <QStringList>
#include <QVariant>

class Subscription : public QObject, public QQmlParserStatus
{
    Q_OBJECT
    QML_ELEMENT
    Q_INTERFACES(QQmlParserStatus)
    Q_PROPERTY(QStringList topics READ topics WRITE setTopics NOTIFY topicsChanged)
    Q_PROPERTY(bool active READ active WRITE setActive NOTIFY activeChanged)

public:
    explicit Subscription(QObject *parent = nullptr);
    ~Subscription() override;

    QStringList topics() const { return m_topics; }
    void setTopics(const QStringList &t);
    bool active() const { return m_active; }
    void setActive(bool a);

    void classBegin() override {}
    void componentComplete() override;

signals:
    void topicsChanged();
    void activeChanged();
    void received(const QString &topic, const QVariant &data);

private:
    void apply(bool on);
    QStringList m_topics;
    bool m_active = true;
    bool m_complete = false;
    bool m_subscribed = false;
};
