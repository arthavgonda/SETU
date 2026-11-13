#pragma once

#include <QElapsedTimer>
#include <QQuickPaintedItem>
#include <QTimer>
#include <QUrl>
#include <memory>

namespace lottie {
struct Composition;
}

class LottieAnimation : public QQuickPaintedItem
{
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(QUrl source READ source WRITE setSource NOTIFY sourceChanged)
    Q_PROPERTY(bool playing READ playing WRITE setPlaying NOTIFY playingChanged)
    Q_PROPERTY(int loops READ loops WRITE setLoops NOTIFY loopsChanged)
    Q_PROPERTY(qreal speed READ speed WRITE setSpeed NOTIFY speedChanged)
    Q_PROPERTY(qreal stillProgress READ stillProgress WRITE setStillProgress NOTIFY stillProgressChanged)
    Q_PROPERTY(qreal progress READ progress NOTIFY progressChanged)
    Q_PROPERTY(int durationMs READ durationMs NOTIFY statusChanged)
    Q_PROPERTY(Status status READ status NOTIFY statusChanged)

public:
    enum Status { Null, Ready, Error };
    Q_ENUM(Status)

    explicit LottieAnimation(QQuickItem *parent = nullptr);
    ~LottieAnimation() override;

    QUrl source() const { return m_source; }
    void setSource(const QUrl &source);
    bool playing() const { return m_playing; }
    void setPlaying(bool playing);
    int loops() const { return m_loops; }
    void setLoops(int loops);
    qreal speed() const { return m_speed; }
    void setSpeed(qreal speed);
    qreal stillProgress() const { return m_still; }
    void setStillProgress(qreal progress);
    qreal progress() const;
    int durationMs() const;
    Status status() const { return m_status; }

    Q_INVOKABLE void restart();
    Q_INVOKABLE void seek(qreal progress);

    void paint(QPainter *painter) override;

    static QImage renderFrame(const QString &path, qreal progress, const QSize &size);

signals:
    void sourceChanged();
    void playingChanged();
    void loopsChanged();
    void speedChanged();
    void stillProgressChanged();
    void progressChanged();
    void statusChanged();
    void finished();

protected:
    void itemChange(ItemChange change, const ItemChangeData &value) override;

private:
    void load();
    void sync();
    void tick();
    void showStill();

    QUrl m_source;
    bool m_playing = true;
    int m_loops = -1;
    int m_done = 0;
    bool m_ended = false;
    qreal m_speed = 1;
    qreal m_still = 0.5;
    double m_frame = 0;
    Status m_status = Null;
    std::shared_ptr<const lottie::Composition> m_comp;
    QTimer m_timer;
    QElapsedTimer m_clock;
    QMetaObject::Connection m_windowWatch;
};
