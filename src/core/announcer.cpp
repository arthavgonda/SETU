#include "announcer.h"

#include <QAccessible>
#include <QGuiApplication>
#include <QQuickWindow>

Announcer::Announcer(QObject *parent)
    : QObject(parent)
{
}

void Announcer::announce(QQuickItem *item, const QString &text, bool assertive) const
{
    if (text.isEmpty())
        return;
    QObject *target = item;
    if (!target)
        target = qGuiApp->focusObject();
    if (!target && !QGuiApplication::topLevelWindows().isEmpty())
        target = QGuiApplication::topLevelWindows().first();
    if (!target)
        return;
    QAccessibleAnnouncementEvent ev(target, text);
    ev.setPoliteness(assertive ? QAccessible::AnnouncementPoliteness::Assertive : QAccessible::AnnouncementPoliteness::Polite);
    QAccessible::updateAccessibility(&ev);
}
