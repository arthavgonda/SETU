#pragma once

#include <QObject>
#include <QQmlEngine>

class Launcher : public QObject
{
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON

public:
    explicit Launcher(QObject *parent = nullptr);

    static QString binary(const QString &name);
    static bool startDaemon();
    Q_INVOKABLE static bool openWindow(const QString &page);
    Q_INVOKABLE static bool restartDaemon();
    Q_INVOKABLE static bool startShell();
    Q_INVOKABLE static QString lastDaemonError();
    Q_INVOKABLE static QString logsFolder();
    Q_INVOKABLE static QString installKind();
    Q_INVOKABLE static QString uninstall();
};
