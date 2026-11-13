#pragma once

#include <QObject>
#include <QQmlEngine>
#include <QQuickWindow>
#include <QRect>
#include <QVariantList>

class OverlayHost : public QObject
{
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(QString mode READ mode CONSTANT)
    Q_PROPERTY(bool fallback READ fallback CONSTANT)
    Q_PROPERTY(QString shots READ shots CONSTANT)

public:
    explicit OverlayHost(QObject *parent = nullptr);

    QString mode() const;
    bool fallback() const;
    QString shots() const;

    Q_INVOKABLE QRect placeTop(QQuickWindow *window, int width, int height);
    Q_INVOKABLE void setShape(QQuickWindow *window, const QVariantList &rects);
    Q_INVOKABLE void raiseNotch(QQuickWindow *window);
    Q_INVOKABLE bool capture(QQuickWindow *window, const QString &path);

    Q_INVOKABLE void prepare(QQuickWindow *window, bool clickThrough);
    Q_INVOKABLE QRect place(QQuickWindow *window, int width, int height, int bottomMargin, int align = 0, int sideMargin = 0);
    Q_INVOKABLE void setInputRegion(QQuickWindow *window, const QVariantList &rects);
    Q_INVOKABLE QRect virtualGeometry() const;

signals:
    void screensChanged();

private:
    void watch(QScreen *screen);
};
