#pragma once

#include <QColor>
#include <QObject>
#include <QVariantMap>
#include <QQmlEngine>

class Platform : public QObject
{
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(bool darkScheme READ darkScheme NOTIFY changed)
    Q_PROPERTY(bool highContrast READ highContrast NOTIFY changed)
    Q_PROPERTY(bool reduceMotion READ reduceMotion NOTIFY changed)
    Q_PROPERTY(qreal osTextScale READ osTextScale NOTIFY changed)
    Q_PROPERTY(QColor highlight READ highlight NOTIFY changed)
    Q_PROPERTY(QColor highlightedText READ highlightedText NOTIFY changed)
    Q_PROPERTY(bool trayAvailable READ trayAvailable NOTIFY changed)
    Q_PROPERTY(QString os READ os CONSTANT)
    Q_PROPERTY(QString desktop READ desktop CONSTANT)
    Q_PROPERTY(bool wayland READ wayland CONSTANT)
    Q_PROPERTY(QString homePath READ homePath CONSTANT)
    Q_PROPERTY(bool rightToLeft READ rightToLeft NOTIFY changed)
    Q_PROPERTY(QString overlayMode READ overlayMode CONSTANT)
    Q_PROPERTY(QVariantMap overrides READ overrides CONSTANT)
    Q_PROPERTY(bool outsideApplications READ outsideApplications CONSTANT)
    Q_PROPERTY(QString userName READ userName CONSTANT)
    Q_PROPERTY(QString userLogin READ userLogin CONSTANT)
    Q_PROPERTY(QString userAvatar READ userAvatar CONSTANT)

public:
    explicit Platform(QObject *parent = nullptr);

    bool darkScheme() const;
    bool highContrast() const;
    bool reduceMotion() const { return m_reduceMotion; }
    qreal osTextScale() const;
    QColor highlight() const;
    QColor highlightedText() const;
    bool trayAvailable() const { return m_tray; }
    QString os() const;
    QString desktop() const;
    bool wayland() const;
    QString homePath() const;
    bool rightToLeft() const;
    QString overlayMode() const;
    QVariantMap overrides() const;
    bool outsideApplications() const;
    QString userName() const;
    QString userLogin() const;
    QString userAvatar() const;

    Q_INVOKABLE void openUrl(const QString &url) const;
    Q_INVOKABLE void openPath(const QString &path) const;
    Q_INVOKABLE void copy(const QString &text) const;
    Q_INVOKABLE QString localFile(const QUrl &url) const;
    Q_INVOKABLE QString formatBytes(qint64 bytes) const;
    Q_INVOKABLE QString formatTime(qint64 ms) const;
    Q_INVOKABLE QString formatDateTime(qint64 ms) const;
    Q_INVOKABLE QString formatDuration(qint64 seconds) const;
    Q_INVOKABLE QString shortcut(const QString &keys) const;
    Q_INVOKABLE QString resourceText(const QString &name) const;
    Q_INVOKABLE qint64 processMs() const;
    Q_INVOKABLE QString moveToApplications() const;

    void readPortal();
    bool eventFilter(QObject *watched, QEvent *event) override;

signals:
    void changed();

private:
    void detectTray();
    void loadProfile() const;
    mutable bool m_profileLoaded = false;
    mutable QString m_userName;
    mutable QString m_userLogin;
    mutable QString m_userAvatar;
    int m_portalScheme = 0;
    qreal m_portalTextScale = 1.0;
    bool m_portalContrast = false;
    bool m_reduceMotion = false;
    bool m_tray = true;
};
