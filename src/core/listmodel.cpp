#include "listmodel.h"

namespace {
constexpr int kModelData = Qt::UserRole + 1;
}

JsonListModel::JsonListModel(QObject *parent)
    : QAbstractListModel(parent)
{
    m_roles.insert(kModelData, "modelData");
}

int JsonListModel::rowCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : int(m_items.size());
}

QVariant JsonListModel::data(const QModelIndex &index, int role) const
{
    if (!index.isValid() || index.row() >= m_items.size())
        return {};
    const QVariantMap m = m_items.at(index.row()).toMap();
    if (role == kModelData)
        return m;
    const auto it = m_roles.constFind(role);
    if (it == m_roles.cend())
        return {};
    return m.value(QString::fromLatin1(it.value()));
}

QHash<int, QByteArray> JsonListModel::roleNames() const
{
    return m_roles;
}

void JsonListModel::rebuildRoles()
{
    int next = kModelData + 1 + int(m_byName.size());
    for (const auto &v : std::as_const(m_items)) {
        const QVariantMap m = v.toMap();
        for (auto it = m.cbegin(); it != m.cend(); ++it) {
            const QByteArray k = it.key().toLatin1();
            if (!m_byName.contains(k)) {
                m_byName.insert(k, next);
                m_roles.insert(next, k);
                ++next;
            }
        }
    }
}

void JsonListModel::setItems(const QVariantList &items)
{
    beginResetModel();
    m_items = items;
    rebuildRoles();
    endResetModel();
    emit itemsChanged();
}

void JsonListModel::setKey(const QString &k)
{
    if (k == m_key)
        return;
    m_key = k;
    emit keyChanged();
}

QVariantMap JsonListModel::get(int row) const
{
    return row >= 0 && row < m_items.size() ? m_items.at(row).toMap() : QVariantMap();
}

int JsonListModel::indexOf(const QVariant &keyValue) const
{
    for (int i = 0; i < m_items.size(); ++i)
        if (m_items.at(i).toMap().value(m_key) == keyValue)
            return i;
    return -1;
}

void JsonListModel::upsert(const QVariantMap &item)
{
    const int i = indexOf(item.value(m_key));
    if (i >= 0) {
        m_items[i] = item;
        const int before = int(m_byName.size());
        rebuildRoles();
        if (int(m_byName.size()) != before) {
            beginResetModel();
            endResetModel();
        } else {
            emit dataChanged(index(i), index(i));
        }
    } else {
        beginInsertRows({}, int(m_items.size()), int(m_items.size()));
        m_items.append(item);
        rebuildRoles();
        endInsertRows();
    }
    emit itemsChanged();
}

void JsonListModel::removeAt(int row)
{
    if (row < 0 || row >= m_items.size())
        return;
    beginRemoveRows({}, row, row);
    m_items.removeAt(row);
    endRemoveRows();
    emit itemsChanged();
}
