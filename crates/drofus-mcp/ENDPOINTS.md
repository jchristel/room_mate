# dRofus API: the read endpoints this crate exposes

Inventory of the **GET** endpoints (and the read-only metadata `OPTIONS` ones) of the dRofus Project API, version 55.1.3, from its OpenAPI definition at `https://api-no.drofus.com/swagger/v1/swagger.json` (public, no sign-in needed to read it). Every path is under `/api/{db}/{pr}` (database, project). Regenerate this list from that file when the API version moves; do not edit it by hand.

73 GET endpoints in 17 groups. Query options on list endpoints: `$select`, `$orderby`, `$filter`, `$top`, `$skip`. `$select` is the only one on `/{id}` endpoints. Endpoints marked **binary** return image bytes (`image/*`, `application/octet-stream`).

## AttributeConfigurations

| Path | What it does | Query | |
|---|---|---|---|
| `/attributeconfigurations` | Get list of Attribute Configurations | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |

## Companies

| Path | What it does | Query | |
|---|---|---|---|
| `/companies` | Get list of Companies | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/companies/{id}` | Get specified Company | `$select` |  |

Read-only metadata (`OPTIONS`, which properties can be selected): `/companies`

## Files

| Path | What it does | Query | |
|---|---|---|---|
| `/files` | Get list of file meta information | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/files/{id}` | Get file meta information by specified id | `$select` |  |

Read-only metadata (`OPTIONS`, which properties can be selected): `/files`

## ItemGroups

| Path | What it does | Query | |
|---|---|---|---|
| `/itemgroups` | Get list of Item Groups | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/itemgroups/{id}` | Get specified Item Group | `$select` |  |

Read-only metadata (`OPTIONS`, which properties can be selected): `/itemgroups`

## Items

| Path | What it does | Query | |
|---|---|---|---|
| `/items` | Get list of Items | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/items/{id}` | Get specified Item | `$select` |  |
| `/items/{id}/files` | Get files meta for specified Item | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/items/{id}/images` | Get list of image metas for a specified Item | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/items/{id}/logs` | Get log entries for specified Item | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/items/{id}/subitems` | Gets list of sub-items for item, if any | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/items/images/{imageid}` | Get specified Item image | - | **binary** |
| `/items/images/{imageid}/meta` | Get specified Item image meta | `$select` |  |
| `/items/logs` | Get log entries for Items | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |

Read-only metadata (`OPTIONS`, which properties can be selected): `/items`

## Occurrences

| Path | What it does | Query | |
|---|---|---|---|
| `/occurrences` | Get list of Occurrences | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/occurrences/{id}` | Get specified Occurrence | `$select` |  |
| `/occurrences/{id}/files` | Get files meta for specified Occurrence | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/occurrences/{id}/images` | Get list of image metas for a specified Occurrence | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/occurrences/{id}/is-member-of-systems` | Gets list of systems Occurrence is member of, if any | `$select`, `$orderby`, `$filter`, `$top`, `$skip`, `includeSubs` |  |
| `/occurrences/{id}/logs` | Get log entries for specified Occurrence | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/occurrences/{id}/statuses/{statustypeid}` | Gets status for given Occurrence ID and status type ID | - |  |
| `/occurrences/images/{imageid}` | Get specified occurrence image | - | **binary** |
| `/occurrences/images/{imageid}/meta` | Get specified Occurrence image meta | `$select` |  |
| `/occurrences/logs` | Get log entries for Occurrences | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |

Read-only metadata (`OPTIONS`, which properties can be selected): `/occurrences`

## Products

| Path | What it does | Query | |
|---|---|---|---|
| `/products` | Get list of Products | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/products/{id}` | Get specified Products | `$select` |  |
| `/products/{id}/files` | Get files meta for specified Product | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |

Read-only metadata (`OPTIONS`, which properties can be selected): `/products`

## Projects

| Path | What it does | Query | |
|---|---|---|---|
| `/projects` | Get current Project | `$select` |  |
| `/projects/largeimage` | Get image configured as Large Project Image | - | **binary** |
| `/projects/largeimageprint` | Get image configured as Large Print Project Image | - | **binary** |
| `/projects/smallimage` | Get image configured as Small Project Image | - | **binary** |
| `/projects/smallimagesecondary` | Get image configured as Small Project Image Secondary | - | **binary** |

Read-only metadata (`OPTIONS`, which properties can be selected): `/projects`

## RoomFunctions

| Path | What it does | Query | |
|---|---|---|---|
| `/roomfunctions` | Get list of Room Functions | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/roomfunctions/{id}` | Get specified Room Function | `$select` |  |
| `/roomfunctions/{id}/files` | Get files meta for specified Room Function | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/roomfunctions/{id}/logs` | Get logs for specified Room Function | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/roomfunctions/logs` | Get logs for all Room Functions | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |

Read-only metadata (`OPTIONS`, which properties can be selected): `/roomfunctions`

## RoomGroups

| Path | What it does | Query | |
|---|---|---|---|
| `/roomgroups` | Get list of Room Group Types | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/roomgroups/{groupTypeId}/members` | Get list of Room Groups in given Room Group Type | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/roomgroups/{id}` | Get specified Room Group Type | `$select` |  |

Read-only metadata (`OPTIONS`, which properties can be selected): `/roomgroups`

## Rooms

| Path | What it does | Query | |
|---|---|---|---|
| `/rooms` | Get list of Rooms | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/rooms/{id}` | Get specified Room | `$select` |  |
| `/rooms/{id}/equipmentliststatus/{equipmentListTypeId}` | Gets the equipment list status  for the given room id and room equipment list type id | - |  |
| `/rooms/{id}/files` | Get files meta for specified Room | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/rooms/{id}/groups/{groupTypeId}` | Get RoomGroup for specified Room and Room Group Type | - |  |
| `/rooms/{id}/images` | Get list of image metas for a specified Room | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/rooms/{id}/logs` | Get log entries for specified Room | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/rooms/{id}/roomdatastatus` | Gets the RDS (Room Data Sheet) status  for the given room id | - |  |
| `/rooms/images/{roomimageid}` | Get specified Room image | - | **binary** |
| `/rooms/images/{roomimageid}/meta` | Get specified Room image meta | `$select` |  |
| `/rooms/logs` | Get log entries for Rooms | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |

Read-only metadata (`OPTIONS`, which properties can be selected): `/rooms`

## RoomTemplates

| Path | What it does | Query | |
|---|---|---|---|
| `/roomtemplates` | Get list of Room Templates | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/roomtemplates/{id}` | Get specified Room Template | `$select` |  |
| `/roomtemplates/{id}/files` | Get files meta for specified Room Template | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/roomtemplates/{id}/logs` | Get logs for specified Room Template | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/roomtemplates/{id}/templateoccurrences` | Get list of Template Occurrences for Room Template | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/roomtemplates/logs` | Get logs for all room templates | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |

Read-only metadata (`OPTIONS`, which properties can be selected): `/roomtemplates`

## SubItems

| Path | What it does | Query | |
|---|---|---|---|
| `/subitems` | Get list of SubItems | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/subitems/{id}` | Get specified SubItem | `$select` |  |

Read-only metadata (`OPTIONS`, which properties can be selected): `/subitems`

## SystemComponents

| Path | What it does | Query | |
|---|---|---|---|
| `/systemcomponents` | Get list of System Components | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/systemcomponents/{id}` | Get specified System Component | `$select` |  |
| `/systemcomponents/{parentId}/components` | Get list of Components in given System Component | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |

Read-only metadata (`OPTIONS`, which properties can be selected): `/systemcomponents`

## Systems

| Path | What it does | Query | |
|---|---|---|---|
| `/systems` | Get list of Systems | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/systems/{id}` | Get specified System | `$select` |  |
| `/systems/{id}/components` | Get components for a specified System | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/systems/{id}/files` | Get files meta for specified System | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/systems/{id}/logs` | Get logs for a specified System | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/systems/logs` | Get logs for all Systems | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |

Read-only metadata (`OPTIONS`, which properties can be selected): `/systems`

## TemplateOccurrences

| Path | What it does | Query | |
|---|---|---|---|
| `/templateoccurrences` | Get list of Template Occurrences | `$select`, `$orderby`, `$filter`, `$top`, `$skip` |  |
| `/templateoccurrences/{id}` | Get specified Template Occurrence. Only root occurrence will be found | `$select` |  |

Read-only metadata (`OPTIONS`, which properties can be selected): `/templateoccurrences`

## Users

| Path | What it does | Query | |
|---|---|---|---|
| `/users/{id}` | Get specified User | - |  |

Read-only metadata (`OPTIONS`, which properties can be selected): `/users`

