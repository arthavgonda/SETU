#pragma once

#include <QElapsedTimer>
#include <QHash>
#include <QJSValue>
#include <QLocalSocket>
#include <QObject>
#include <QQmlEngine>
#include <QTimer>
#include <QVariantMap>
#include <functional>

class DaemonClient : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(Daemon)
    QML_SINGLETON
    Q_PROPERTY(QString status READ status NOTIFY statusChanged)
    Q_PROPERTY(bool connected READ connected NOTIFY statusChanged)
    Q_PROPERTY(QString lastError READ lastError NOTIFY statusChanged)
    Q_PROPERTY(int attempts READ attempts NOTIFY statusChanged)
    Q_PROPERTY(QVariantMap daemon READ daemon NOTIFY daemonChanged)
    Q_PROPERTY(QVariantMap mic READ mic NOTIFY micChanged)
    Q_PROPERTY(QVariantMap session READ session NOTIFY sessionChanged)
    Q_PROPERTY(QVariantList attention READ attention NOTIFY attentionChanged)
    Q_PROPERTY(QVariantMap update READ update NOTIFY updateChanged)
    Q_PROPERTY(QVariantMap paths READ paths NOTIFY pathsChanged)
    Q_PROPERTY(QVariantList models READ models NOTIFY modelsChanged)
    Q_PROPERTY(QString daemonVersion READ daemonVersion NOTIFY pathsChanged)
    Q_PROPERTY(int manifestVersion READ manifestVersion NOTIFY pathsChanged)
    Q_PROPERTY(bool speakerEnrolled READ speakerEnrolled NOTIFY pathsChanged)

public:
    using Callback = std::function<void(bool ok, const QVariant &result, const QVariantMap &error)>;

    explicit DaemonClient(const QString &role, QObject *parent = nullptr);
    ~DaemonClient() override;

    static DaemonClient *instance();
    static DaemonClient *create(QQmlEngine *, QJSEngine *);
    static QString socketPath();
    static bool ownedByCurrentUser(const QString &path);

    void start();
    void startWhenMissing(bool withShell);
    void request(const QString &method, const QVariantMap &params, Callback cb);

    Q_INVOKABLE void call(const QString &method, const QVariantMap &params = {}, const QJSValue &callback = QJSValue());
    Q_INVOKABLE void subscribe(const QStringList &topics);
    Q_INVOKABLE void unsubscribe(const QStringList &topics);
    Q_INVOKABLE void reconnectNow();
    Q_INVOKABLE void startDaemon();
    Q_INVOKABLE QVariantMap model(const QString &slot) const;

    QString status() const { return m_status; }
    bool connected() const { return m_status == QLatin1String("connected"); }
    QString lastError() const { return m_lastError; }
    int attempts() const { return m_attempts; }
    QVariantMap daemon() const { return m_daemon; }
    QVariantMap mic() const { return m_mic; }
    QVariantMap session() const { return m_session; }
    QVariantList attention() const { return m_attention; }
    QVariantMap update() const { return m_update; }
    QVariantMap paths() const { return m_paths; }
    QVariantList models() const { return m_models; }
    QString daemonVersion() const { return m_version; }
    int manifestVersion() const { return m_manifest; }
    bool speakerEnrolled() const { return m_speaker; }
    QVariantMap settings() const { return m_settings; }

signals:
    void statusChanged();
    void daemonChanged();
    void micChanged();
    void sessionChanged();
    void attentionChanged();
    void updateChanged();
    void pathsChanged();
    void modelsChanged();
    void settingsLoaded(const QVariantMap &settings);
    void settingsChanged(const QVariantMap &changes);
    void event(const QString &topic, const QVariant &data);
    void reconnected();

private:
    struct Pending {
        Callback cb;
        qint64 deadline;
    };
    struct Queued {
        QString method;
        QVariantMap params;
        Callback cb;
        qint64 deadline;
    };

    void setStatus(const QString &s, const QString &error = {});
    void connectNow();
    void onConnected();
    void onReadyRead();
    void onDisconnected();
    void scheduleRetry();
    bool settling() const;
    void send(const QJsonObject &obj);
    void handle(const QJsonObject &obj);
    void handleEvent(const QString &topic, const QVariant &data);
    void applySnapshot(const QVariantMap &snap);
    void armTimeouts();
    void flushBacklog();
    void failBacklog(const QString &code, const QString &message, bool expiredOnly);
    void expire();
    void failAll(const QString &code, const QString &message);
    qint64 timeoutFor(const QString &method) const;

    QString m_role;
    QLocalSocket m_socket;
    QByteArray m_buffer;
    QHash<QString, Pending> m_pending;
    QHash<QString, int> m_subs;
    QTimer m_retry;
    QTimer m_timeouts;
    QElapsedTimer m_clock;
    quint64 m_next = 0;
    int m_attempts = 0;
    int m_backoff = 250;
    bool m_helloDone = false;
    int m_autoStart = 0;
    qint64 m_startedAt = -1;
    QString m_status = QStringLiteral("connecting");
    QList<Queued> m_backlog;
    QString m_lastError;
    QVariantMap m_daemon, m_mic, m_session, m_update, m_paths, m_settings;
    QVariantList m_attention, m_models;
    QString m_version;
    int m_manifest = 0;
    bool m_speaker = false;
};
