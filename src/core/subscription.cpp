#include "subscription.h"
#include "daemonclient.h"

Subscription::Subscription(QObject *parent)
    : QObject(parent)
{
    if (auto *d = DaemonClient::instance()) {
        connect(d, &DaemonClient::event, this, [this](const QString &topic, const QVariant &data) {
            if (m_subscribed && m_topics.contains(topic))
                emit received(topic, data);
        });
    }
}

Subscription::~Subscription()
{
    apply(false);
}

void Subscription::setTopics(const QStringList &t)
{
    if (t == m_topics)
        return;
    const bool was = m_subscribed;
    apply(false);
    m_topics = t;
    emit topicsChanged();
    if (was || (m_complete && m_active))
        apply(true);
}

void Subscription::setActive(bool a)
{
    if (a == m_active)
        return;
    m_active = a;
    emit activeChanged();
    if (m_complete)
        apply(a);
}

void Subscription::componentComplete()
{
    m_complete = true;
    if (m_active)
        apply(true);
}

void Subscription::apply(bool on)
{
    auto *d = DaemonClient::instance();
    if (!d || on == m_subscribed || m_topics.isEmpty())
        return;
    m_subscribed = on;
    if (on)
        d->subscribe(m_topics);
    else
        d->unsubscribe(m_topics);
}
