#pragma once

#include <QObject>
#include <QQmlEngine>
#include <QTranslator>
#include <QVariantList>
#include <memory>

class Translator : public QObject
{
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(QString language READ language NOTIFY languageChanged)
    Q_PROPERTY(QVariantList languages READ languages CONSTANT)

public:
    explicit Translator(QObject *parent = nullptr);
    static Translator *instance();
    static Translator *create(QQmlEngine *, QJSEngine *);

    void setEngine(QQmlEngine *engine) { m_engine = engine; }
    QString language() const { return m_language; }
    QVariantList languages() const;
    Q_INVOKABLE void apply(const QString &language);

signals:
    void languageChanged();

private:
    QQmlEngine *m_engine = nullptr;
    QString m_language = QStringLiteral("en");
    std::unique_ptr<QTranslator> m_translator;
};
