#pragma once

#include <QObject>
#include <QQmlEngine>
#include <QQuickItem>

class Announcer : public QObject
{
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON

public:
    explicit Announcer(QObject *parent = nullptr);
    Q_INVOKABLE void announce(QQuickItem *item, const QString &text, bool assertive = false) const;
};
