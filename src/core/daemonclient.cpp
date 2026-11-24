#include "daemonclient.h"
#include "launcher.h"

#include <QCoreApplication>
#include <QDir>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QStandardPaths>
#include <QtEndian>

#ifdef Q_OS_UNIX
#include <unistd.h>
#endif

namespace {
DaemonClient *g_instance = nullptr;
constexpr quint32 kMaxFrame = 1u << 20;
const QStringList kAlways = {
    QStringLiteral("state.daemon"), QStringLiteral("state.mic"), QStringLiteral("session.state"), QStringLiteral("attention.changed"),
    QStringLiteral("settings.changed"), QStringLiteral("update.state"), QStringLiteral("models.state"),
};
const QStringList kLong = {
    QStringLiteral("models.download"), QStringLiteral("models.warm"), QStringLiteral("models.use"), QStringLiteral("onboarding.paragraph"), QStringLiteral("speaker.enroll"), QStringLiteral("models.testConnection"), QStringLiteral("benchmark.run"), QStringLiteral("history.export"),
    QStringLiteral("data.export"), QStringLiteral("diagnostics.export"), QStringLiteral("apps.rescan"), QStringLiteral("update.check"),
    QStringLiteral("wifi.scan"), QStringLiteral("wifi.connect"), QStringLiteral("apps.install"), QStringLiteral("data.wipe"),
    QStringLiteral("skills.state"),
};
}

DaemonClient::DaemonClient(const QString &role, QObject *parent)
    : QObject(parent), m_role(role)
{
    g_instance = this;
    m_clock.start();
    m_retry.setSingleShot(true);
    m_timeouts.setSingleShot(true);
    connect(&m_retry, &QTimer::timeout, this, &DaemonClient::connectNow);
    connect(&m_timeouts, &QTimer::timeout, this, &DaemonClient::expire);
    connect(&m_socket, &QLocalSocket::connected, this, &DaemonClient::onConnected);
    connect(&m_socket, &QLocalSocket::readyRead, this, &DaemonClient::onReadyRead);
    connect(&m_socket, &QLocalSocket::disconnected, this, &DaemonClient::onDisconnected);
    connect(&m_socket, &QLocalSocket::errorOccurred, this, [this](QLocalSocket::LocalSocketError) {
        if (m_socket.state() != QLocalSocket::ConnectedState)
            onDisconnected();
    });
}

DaemonClient::~DaemonClient()
{
    m_socket.disconnect(this);
    m_retry.stop();
    m_pending.clear();
    m_backlog.clear();
    if (g_instance == this)
        g_instance = nullptr;
}

DaemonClient *DaemonClient::instance()
{
    return g_instance;
}

DaemonClient *DaemonClient::create(QQmlEngine *, QJSEngine *)
{
    QJSEngine::setObjectOwnership(g_instance, QJSEngine::CppOwnership);
    return g_instance;
}

QString DaemonClient::socketPath()
{
    const QByteArray over = qgetenv("SETU_RUNTIME_DIR");
    if (!over.isEmpty())
        return QDir(QString::fromLocal8Bit(over)).filePath(QStringLiteral("daemon.sock"));
#if defined(Q_OS_WIN)
    return QStringLiteral("setu-%1").arg(qEnvironmentVariable("USERNAME", QStringLiteral("user")));
#elif defined(Q_OS_MACOS)
    return QDir(QDir::tempPath()).filePath(QStringLiteral("setu/daemon.sock"));
#else
    const QString rt = qEnvironmentVariable("XDG_RUNTIME_DIR");
    if (!rt.isEmpty())
        return QDir(rt).filePath(QStringLiteral("setu/daemon.sock"));
    return QStringLiteral("/tmp/setu-%1/daemon.sock").arg(getuid());
#endif
}

bool DaemonClient::ownedByCurrentUser(const QString &path)
{
#ifdef Q_OS_UNIX
    const QFileInfo fi(path);
    if (!fi.exists())
        return true;
    return fi.ownerId() == getuid();
#else
    Q_UNUSED(path)
    return true;
#endif
}

void DaemonClient::start()
{
    connectNow();
}

void DaemonClient::startWhenMissing(bool withShell)
{
    m_autoStart = withShell ? 2 : 1;
}

bool DaemonClient::settling() const
{
    return m_startedAt >= 0 && m_clock.elapsed() - m_startedAt < 20000;
}

void DaemonClient::setStatus(const QString &s, const QString &error)
{
    if (m_status == s && m_lastError == error)
        return;
    m_status = s;
    m_lastError = error;
    if (s != QLatin1String("connecting") && s != QLatin1String("connected"))
        failBacklog(QStringLiteral("unavailable"), QStringLiteral("The background service is not running."), false);
    emit statusChanged();
}

void DaemonClient::connectNow()
{
    if (m_socket.state() != QLocalSocket::UnconnectedState)
        return;
    const QString path = socketPath();
    if (!ownedByCurrentUser(path)) {
        setStatus(QStringLiteral("refused"), QStringLiteral("The service socket belongs to another user."));
        return;
    }
    ++m_attempts;
    m_helloDone = false;
    m_buffer.clear();
    m_socket.connectToServer(path);
}

void DaemonClient::reconnectNow()
{
    m_retry.stop();
    m_backoff = 250;
    connectNow();
}

void DaemonClient::startDaemon()
{
    Launcher::startDaemon();
    m_startedAt = m_clock.elapsed();
    m_backoff = 250;
    m_retry.start(400);
}

void DaemonClient::scheduleRetry()
{
    if (m_status == QLatin1String("mismatch") || m_status == QLatin1String("refused"))
        return;
    m_retry.start(m_backoff);
    m_backoff = qMin(m_backoff * 2, settling() ? 1000 : 5000);
}

void DaemonClient::onConnected()
{
    QVariantMap p;
    p[QStringLiteral("client")] = QCoreApplication::applicationName();
    p[QStringLiteral("version")] = QCoreApplication::applicationVersion();
    p[QStringLiteral("role")] = m_role;
    p[QStringLiteral("protocol")] = 1;
    const QString id = QStringLiteral("c-%1").arg(++m_next);
    QJsonObject o{{QStringLiteral("v"), 1}, {QStringLiteral("id"), id}, {QStringLiteral("type"), QStringLiteral("request")},
                  {QStringLiteral("method"), QStringLiteral("hello")}, {QStringLiteral("params"), QJsonObject::fromVariantMap(p)}};
    m_pending.insert(id, {[this](bool ok, const QVariant &result, const QVariantMap &error) {
                              if (!ok) {
                                  if (error.value(QStringLiteral("code")) == QLatin1String("version_mismatch")) {
                                      setStatus(QStringLiteral("mismatch"), error.value(QStringLiteral("message")).toString());
                                      m_socket.abort();
                                  }
                                  return;
                              }
                              const QVariantMap r = result.toMap();
                              m_version = r.value(QStringLiteral("daemon")).toMap().value(QStringLiteral("version")).toString();
                              m_helloDone = true;
                              m_autoStart = 0;
                              m_startedAt = -1;
                              m_backoff = 250;
                              m_attempts = 0;
                              applySnapshot(r.value(QStringLiteral("snapshot")).toMap());
                              QStringList topics = kAlways;
                              for (auto it = m_subs.cbegin(); it != m_subs.cend(); ++it)
                                  if (it.value() > 0 && !topics.contains(it.key()))
                                      topics << it.key();
                              QJsonObject sub{{QStringLiteral("v"), 1}, {QStringLiteral("id"), QStringLiteral("c-%1").arg(++m_next)},
                                              {QStringLiteral("type"), QStringLiteral("request")}, {QStringLiteral("method"), QStringLiteral("events.subscribe")},
                                              {QStringLiteral("params"), QJsonObject{{QStringLiteral("topics"), QJsonArray::fromStringList(topics)}}}};
                              send(sub);
                              setStatus(QStringLiteral("connected"));
                              flushBacklog();
                              emit reconnected();
                          },
                          m_clock.elapsed() + 5000});
    send(o);
    armTimeouts();
}

void DaemonClient::applySnapshot(const QVariantMap &snap)
{
    m_daemon = snap.value(QStringLiteral("daemon")).toMap();
    m_mic = snap.value(QStringLiteral("mic")).toMap();
    m_session = snap.value(QStringLiteral("session")).toMap();
    m_attention = snap.value(QStringLiteral("attention")).toList();
    m_update = snap.value(QStringLiteral("update")).toMap();
    m_paths = snap.value(QStringLiteral("paths")).toMap();
    m_models = snap.value(QStringLiteral("models")).toList();
    m_settings = snap.value(QStringLiteral("settings")).toMap();
    m_manifest = snap.value(QStringLiteral("manifestVersion")).toInt();
    m_speaker = snap.value(QStringLiteral("speaker")).toMap().value(QStringLiteral("enrolled")).toBool();
    emit daemonChanged();
    emit micChanged();
    emit sessionChanged();
    emit attentionChanged();
    emit updateChanged();
    emit pathsChanged();
    emit modelsChanged();
    emit settingsLoaded(m_settings);
}

void DaemonClient::onDisconnected()
{
    const bool was = connected();
    m_helloDone = false;
    failAll(QStringLiteral("unavailable"), QStringLiteral("The background service is not running."));
    if (m_status == QLatin1String("mismatch") || m_status == QLatin1String("refused"))
        return;
    if (m_autoStart && !was) {
        const bool shell = m_autoStart == 2;
        m_autoStart = 0;
        Launcher::startDaemon();
        if (shell)
            Launcher::startShell();
        m_startedAt = m_clock.elapsed();
        m_backoff = 250;
    }
    if (was || m_status == QLatin1String("connecting") || m_status == QLatin1String("stopped"))
        setStatus(m_attempts >= 3 && !settling() ? QStringLiteral("stopped") : QStringLiteral("connecting"));
    if (m_socket.state() != QLocalSocket::UnconnectedState)
        m_socket.abort();
    scheduleRetry();
}

void DaemonClient::onReadyRead()
{
    m_buffer.append(m_socket.readAll());
    while (m_buffer.size() >= 4) {
        const quint32 len = qFromBigEndian<quint32>(reinterpret_cast<const uchar *>(m_buffer.constData()));
        if (len == 0 || len > kMaxFrame) {
            m_socket.abort();
            return;
        }
        if (quint32(m_buffer.size()) < 4 + len)
            return;
        const QByteArray body = m_buffer.mid(4, int(len));
        m_buffer.remove(0, int(len) + 4);
        QJsonParseError err{};
        const QJsonDocument doc = QJsonDocument::fromJson(body, &err);
        if (err.error != QJsonParseError::NoError || !doc.isObject()) {
            m_socket.abort();
            return;
        }
        handle(doc.object());
    }
}

void DaemonClient::handle(const QJsonObject &obj)
{
    const QString type = obj.value(QStringLiteral("type")).toString();
    if (type == QLatin1String("response")) {
        const QString id = obj.value(QStringLiteral("id")).toString();
        auto it = m_pending.find(id);
        if (it == m_pending.end())
            return;
        Pending p = it.value();
        m_pending.erase(it);
        armTimeouts();
        const bool ok = obj.value(QStringLiteral("ok")).toBool();
        if (p.cb)
            p.cb(ok, obj.value(QStringLiteral("result")).toVariant(), obj.value(QStringLiteral("error")).toObject().toVariantMap());
    } else if (type == QLatin1String("event")) {
        handleEvent(obj.value(QStringLiteral("topic")).toString(), obj.value(QStringLiteral("data")).toVariant());
    } else {
        m_socket.abort();
    }
}

void DaemonClient::handleEvent(const QString &topic, const QVariant &data)
{
    const QVariantMap d = data.toMap();
    if (topic == QLatin1String("state.daemon")) {
        m_daemon = d;
        emit daemonChanged();
    } else if (topic == QLatin1String("state.mic")) {
        m_mic = d;
        emit micChanged();
    } else if (topic == QLatin1String("session.state")) {
        m_session = d;
        emit sessionChanged();
    } else if (topic == QLatin1String("attention.changed")) {
        m_attention = d.value(QStringLiteral("issues")).toList();
        emit attentionChanged();
    } else if (topic == QLatin1String("update.state")) {
        m_update = d;
        emit updateChanged();
    } else if (topic == QLatin1String("settings.changed")) {
        const QVariantMap c = d.value(QStringLiteral("changes")).toMap();
        for (auto it = c.cbegin(); it != c.cend(); ++it)
            m_settings.insert(it.key(), it.value());
        emit settingsChanged(c);
    } else if (topic == QLatin1String("models.state")) {
        const QString slot = d.value(QStringLiteral("slot")).toString();
        bool found = false;
        for (auto &m : m_models) {
            if (m.toMap().value(QStringLiteral("slot")) == slot) {
                m = d;
                found = true;
            }
        }
        if (!found)
            m_models.append(d);
        emit modelsChanged();
    }
    emit event(topic, data);
}

QVariantMap DaemonClient::model(const QString &slot) const
{
    for (const auto &m : m_models)
        if (m.toMap().value(QStringLiteral("slot")) == slot)
            return m.toMap();
    return {};
}

void DaemonClient::send(const QJsonObject &obj)
{
    const QByteArray body = QJsonDocument(obj).toJson(QJsonDocument::Compact);
    if (body.size() > int(kMaxFrame))
        return;
    QByteArray frame(4, '\0');
    qToBigEndian<quint32>(quint32(body.size()), reinterpret_cast<uchar *>(frame.data()));
    frame.append(body);
    m_socket.write(frame);
}

qint64 DaemonClient::timeoutFor(const QString &method) const
{
    if (method == QLatin1String("models.warm"))
        return 180000;
    return kLong.contains(method) ? 60000 : 5000;
}

void DaemonClient::request(const QString &method, const QVariantMap &params, Callback cb)
{
    if (!connected() || !m_helloDone) {
        if (m_status == QLatin1String("connecting")) {
            m_backlog.append({method, params, std::move(cb), m_clock.elapsed() + 6000});
            armTimeouts();
            return;
        }
        if (cb) {
            QVariantMap e{{QStringLiteral("code"), QStringLiteral("unavailable")}, {QStringLiteral("message"), QStringLiteral("The background service is not running.")}};
            QTimer::singleShot(0, this, [cb, e] { cb(false, {}, e); });
        }
        return;
    }
    const QString id = QStringLiteral("c-%1").arg(++m_next);
    m_pending.insert(id, {std::move(cb), m_clock.elapsed() + timeoutFor(method)});
    send(QJsonObject{{QStringLiteral("v"), 1}, {QStringLiteral("id"), id}, {QStringLiteral("type"), QStringLiteral("request")},
                     {QStringLiteral("method"), method}, {QStringLiteral("params"), QJsonObject::fromVariantMap(params)}});
    armTimeouts();
}

void DaemonClient::call(const QString &method, const QVariantMap &params, const QJSValue &callback)
{
    QJSValue cb = callback;
    request(method, params, [cb](bool ok, const QVariant &result, const QVariantMap &error) mutable {
        if (!cb.isCallable())
            return;
        QJSEngine *e = qjsEngine(g_instance);
        if (!e)
            return;
        cb.call({QJSValue(ok), e->toScriptValue(result), e->toScriptValue(error)});
    });
}

void DaemonClient::flushBacklog()
{
    const auto queued = std::exchange(m_backlog, {});
    for (const auto &q : queued)
        request(q.method, q.params, q.cb);
}

void DaemonClient::failBacklog(const QString &code, const QString &message, bool expiredOnly)
{
    const qint64 now = m_clock.elapsed();
    QList<Callback> dead;
    for (auto it = m_backlog.begin(); it != m_backlog.end();) {
        if (!expiredOnly || it->deadline <= now) {
            dead << it->cb;
            it = m_backlog.erase(it);
        } else {
            ++it;
        }
    }
    const QVariantMap e{{QStringLiteral("code"), code}, {QStringLiteral("message"), message}};
    for (auto &cb : dead)
        if (cb)
            cb(false, {}, e);
}

void DaemonClient::armTimeouts()
{
    if (m_pending.isEmpty() && m_backlog.isEmpty()) {
        m_timeouts.stop();
        return;
    }
    qint64 next = std::numeric_limits<qint64>::max();
    for (const auto &p : std::as_const(m_pending))
        next = qMin(next, p.deadline);
    for (const auto &q : std::as_const(m_backlog))
        next = qMin(next, q.deadline);
    m_timeouts.start(int(qMax<qint64>(0, next - m_clock.elapsed())));
}

void DaemonClient::expire()
{
    const qint64 now = m_clock.elapsed();
    QList<Callback> dead;
    for (auto it = m_pending.begin(); it != m_pending.end();) {
        if (it.value().deadline <= now) {
            dead << it.value().cb;
            it = m_pending.erase(it);
        } else {
            ++it;
        }
    }
    const QVariantMap e{{QStringLiteral("code"), QStringLiteral("timeout")}, {QStringLiteral("message"), QStringLiteral("The background service did not answer in time.")}};
    for (auto &cb : dead)
        if (cb)
            cb(false, {}, e);
    failBacklog(QStringLiteral("unavailable"), QStringLiteral("The background service is not running."), true);
    armTimeouts();
}

void DaemonClient::failAll(const QString &code, const QString &message)
{
    const auto pending = std::exchange(m_pending, {});
    const QVariantMap e{{QStringLiteral("code"), code}, {QStringLiteral("message"), message}};
    for (const auto &p : pending)
        if (p.cb)
            p.cb(false, {}, e);
    m_timeouts.stop();
}

void DaemonClient::subscribe(const QStringList &topics)
{
    QStringList fresh;
    for (const auto &t : topics) {
        if (++m_subs[t] == 1 && !kAlways.contains(t))
            fresh << t;
    }
    if (!fresh.isEmpty() && connected())
        request(QStringLiteral("events.subscribe"), {{QStringLiteral("topics"), fresh}}, {});
}

void DaemonClient::unsubscribe(const QStringList &topics)
{
    QStringList gone;
    for (const auto &t : topics) {
        auto it = m_subs.find(t);
        if (it == m_subs.end())
            continue;
        if (--it.value() <= 0) {
            m_subs.erase(it);
            if (!kAlways.contains(t))
                gone << t;
        }
    }
    if (!gone.isEmpty() && connected())
        request(QStringLiteral("events.unsubscribe"), {{QStringLiteral("topics"), gone}}, {});
}
