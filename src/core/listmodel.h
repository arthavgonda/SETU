#pragma once

#include <QAbstractListModel>
#include <QQmlEngine>
#include <QVariantList>

class JsonListModel : public QAbstractListModel
{
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(QVariantList items READ items WRITE setItems NOTIFY itemsChanged)
    Q_PROPERTY(int count READ count NOTIFY itemsChanged)
    Q_PROPERTY(QString key READ key WRITE setKey NOTIFY keyChanged)

public:
    explicit JsonListModel(QObject *parent = nullptr);

    int rowCount(const QModelIndex &parent = {}) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    QVariantList items() const { return m_items; }
    void setItems(const QVariantList &items);
    int count() const { return int(m_items.size()); }
    QString key() const { return m_key; }
    void setKey(const QString &k);

    Q_INVOKABLE QVariantMap get(int row) const;
    Q_INVOKABLE int indexOf(const QVariant &keyValue) const;
    Q_INVOKABLE void upsert(const QVariantMap &item);
    Q_INVOKABLE void removeAt(int row);

signals:
    void itemsChanged();
    void keyChanged();

private:
    void rebuildRoles();
    QVariantList m_items;
    QString m_key = QStringLiteral("id");
    QHash<int, QByteArray> m_roles;
    QHash<QByteArray, int> m_byName;
};
