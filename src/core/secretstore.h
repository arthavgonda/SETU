#pragma once

#include <QJSValue>
#include <QObject>
#include <QQmlEngine>

class SecretStore : public QObject
{
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON

public:
    explicit SecretStore(QObject *parent = nullptr);

    static bool validRef(const QString &ref);
    static QString storeSync(const QString &ref, const QString &secret);
    static QString removeSync(const QString &ref);
    static bool hasSync(const QString &ref);

    Q_INVOKABLE void store(const QString &ref, const QString &secret, const QJSValue &callback = QJSValue());
    Q_INVOKABLE void remove(const QString &ref, const QJSValue &callback = QJSValue());
    Q_INVOKABLE void has(const QString &ref, const QJSValue &callback);

private:
    void finish(QJSValue cb, const QJSValueList &args);
};
