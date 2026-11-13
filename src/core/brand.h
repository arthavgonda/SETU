#pragma once

#include <QObject>
#include <QQmlEngine>

class Brand : public QObject
{
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(QString productName READ productName CONSTANT)
    Q_PROPERTY(QString fullName READ fullName CONSTANT)
    Q_PROPERTY(QString version READ version CONSTANT)
    Q_PROPERTY(QString build READ build CONSTANT)
    Q_PROPERTY(QString assistantName READ assistantName NOTIFY assistantChanged)
    Q_PROPERTY(QString wakePhrase READ wakePhrase NOTIFY assistantChanged)
    Q_PROPERTY(QString supportUrl READ supportUrl CONSTANT)
    Q_PROPERTY(QString feedbackAddress READ feedbackAddress CONSTANT)

public:
    explicit Brand(QObject *parent = nullptr);

    QString productName() const;
    QString fullName() const;
    QString version() const;
    QString build() const;
    QString assistantName() const;
    QString wakePhrase() const;
    QString supportUrl() const;
    QString feedbackAddress() const;

signals:
    void assistantChanged();
};
