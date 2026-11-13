#pragma once

#include <QCache>
#include <QMutex>
#include <QQuickImageProvider>

class IconProvider : public QQuickImageProvider
{
public:
    IconProvider();
    QImage requestImage(const QString &id, QSize *size, const QSize &requested) override;

private:
    QMutex m_lock;
    QCache<QString, QImage> m_cache;
};

class TrayIconProvider : public QQuickImageProvider
{
public:
    TrayIconProvider();
    QImage requestImage(const QString &id, QSize *size, const QSize &requested) override;
};

class AppIconProvider : public QQuickImageProvider
{
public:
    AppIconProvider();
    QImage requestImage(const QString &id, QSize *size, const QSize &requested) override;

private:
    QMutex m_lock;
    QCache<QString, QImage> m_cache;
};
