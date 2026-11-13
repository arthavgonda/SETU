#pragma once

#include <QAbstractListModel>
#include <QQmlEngine>
#include <QVariantMap>

class HistoryModel : public QAbstractListModel
{
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(QVariantMap filters READ filters WRITE setFilters NOTIFY filtersChanged)
    Q_PROPERTY(QString sortKey READ sortKey WRITE setSortKey NOTIFY sortChanged)
    Q_PROPERTY(bool sortDesc READ sortDesc WRITE setSortDesc NOTIFY sortChanged)
    Q_PROPERTY(int total READ total NOTIFY stateChanged)
    Q_PROPERTY(int count READ count NOTIFY stateChanged)
    Q_PROPERTY(bool loading READ loading NOTIFY stateChanged)
    Q_PROPERTY(bool hasMore READ hasMore NOTIFY stateChanged)
    Q_PROPERTY(QString error READ error NOTIFY stateChanged)
    Q_PROPERTY(int pageSize READ pageSize WRITE setPageSize NOTIFY pageSizeChanged)

public:
    enum Roles { IdRole = Qt::UserRole + 1, TimeRole, SaidRole, DidRole, ResultRole, LayerRole, DurationRole, AppRole, ActionRole, UndoableRole, ItemRole };

    explicit HistoryModel(QObject *parent = nullptr);

    int rowCount(const QModelIndex &parent = {}) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;
    bool canFetchMore(const QModelIndex &parent) const override;
    void fetchMore(const QModelIndex &parent) override;

    QVariantMap filters() const { return m_filters; }
    void setFilters(const QVariantMap &f);
    QString sortKey() const { return m_sortKey; }
    void setSortKey(const QString &k);
    bool sortDesc() const { return m_desc; }
    void setSortDesc(bool d);
    int total() const { return m_total; }
    int count() const { return int(m_rows.size()); }
    bool loading() const { return m_loading; }
    bool hasMore() const { return !m_cursor.isEmpty(); }
    QString error() const { return m_error; }
    int pageSize() const { return m_page; }
    void setPageSize(int n);

    Q_INVOKABLE void reload();
    Q_INVOKABLE void loadMore();
    Q_INVOKABLE QVariantMap get(int row) const;
    Q_INVOKABLE void prepend(const QVariantMap &item);
    Q_INVOKABLE void removeIds(const QStringList &ids);

signals:
    void filtersChanged();
    void sortChanged();
    void stateChanged();
    void pageSizeChanged();

private:
    void fetch(bool reset);
    QVariantList m_rows;
    QVariantMap m_filters;
    QString m_sortKey = QStringLiteral("time");
    bool m_desc = true;
    QString m_cursor;
    int m_total = 0;
    bool m_loading = false;
    QString m_error;
    quint64 m_generation = 0;
    int m_page = 200;
};
