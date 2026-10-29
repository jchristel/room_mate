"""RoomMate settings kept INSIDE the Revit project file, in extensible storage.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. The Revit
calls are made inside the functions that need them, so the pure half (`parse`, `with_server`,
`server_of`) is tested on CPython (`extractor/tests/test_project_settings.py`).

**One text field holding one JSON object**, so a setting added later is a new key and not a new
schema field: a schema is fixed once a file has been saved with it, and changing it afterwards
means migrating every project (the pattern of duHast's own pyRevit settings, which this follows:
`ExtensibleSchemas`, a `DataStorage` element, `Get[str]` / `Set[str]`). The JSON carries a
`version`, and `server` is the first key: the URL of the RoomMate server for this model.

**The server address here is shared by everyone who opens the model**, and travels with it
(a detached copy, a model sent to another office). That is the point -- nobody has to set it up
again -- and also why it is only a SECOND source: `room_m.server_config` reads it before the
per-computer `server.json`, and the Server button writes both. Reading never opens a transaction
and never modifies the model. Writing does, so it happens only when the Server button is pressed.
"""

import json

SCHEMA_GUID = "198d3948-6b87-45f7-ad6a-8e285fb9f99c"
SCHEMA_NAME = "RoomMateProjectSettings"
SCHEMA_DOCUMENTATION = "RoomMate settings for this model: one JSON object in a single text field."
FIELD = "json"
VERSION = 1


def parse(text):
    """The settings object in `text`, or an empty one when `text` is absent or not a JSON object."""
    if not text:
        return {}
    try:
        data = json.loads(text)
    except (TypeError, ValueError):
        return {}
    return data if isinstance(data, dict) else {}


def server_of(text):
    """The server address kept in `text`, or None."""
    value = parse(text).get("server")
    return value if isinstance(value, (type(u""), str)) and value.strip() else None


def with_server(text, url):
    """`text` with the server address set to `url` (removed when `url` is None). Every other key is
    kept, so a setting written by a later version survives this one saving."""
    data = parse(text)
    data["version"] = max(VERSION, data.get("version") if isinstance(data.get("version"), int) else 0)
    if url:
        data["server"] = url
    else:
        data.pop("server", None)
    return json.dumps(data, sort_keys=True)


def _find_storage(doc, schema):
    from Autodesk.Revit.DB import FilteredElementCollector
    from Autodesk.Revit.DB.ExtensibleStorage import DataStorage

    for storage in FilteredElementCollector(doc).OfClass(DataStorage):
        entity = storage.GetEntity(schema)
        if entity is not None and entity.IsValid():
            return storage
    return None


def read_text(doc):
    """The JSON text stored in `doc`, or None when the model has none (no schema, no storage
    element, or no document). Never raises: the caller falls through to the next source."""
    if doc is None:
        return None
    try:
        from System import Guid
        from Autodesk.Revit.DB.ExtensibleStorage import Schema

        schema = Schema.Lookup(Guid(SCHEMA_GUID))
        if schema is None:
            return None
        storage = _find_storage(doc, schema)
        if storage is None:
            return None
        return storage.GetEntity(schema).Get[str](FIELD)
    except Exception:
        return None


def write_text(doc, text):
    """Store `text` in `doc`, creating the schema and the storage element on first use. Returns
    `(ok, message)`; a model that cannot take it (read only, a family, a link) says so."""
    if doc is None:
        return False, "there is no open model to store it in"
    try:
        if doc.IsFamilyDocument:
            return False, "a family document cannot hold RoomMate settings"
        if doc.IsReadOnly:
            return False, "the model is read only"
        import clr
        from System import Guid
        from Autodesk.Revit.DB import Transaction
        from Autodesk.Revit.DB.ExtensibleStorage import AccessLevel, DataStorage, Entity, Schema, SchemaBuilder

        schema = Schema.Lookup(Guid(SCHEMA_GUID))
        transaction = Transaction(doc, "RoomMate: store settings")
        transaction.Start()
        try:
            if schema is None:
                builder = SchemaBuilder(Guid(SCHEMA_GUID))
                builder.SetReadAccessLevel(AccessLevel.Public)
                builder.SetWriteAccessLevel(AccessLevel.Public)
                builder.SetSchemaName(SCHEMA_NAME)
                builder.SetDocumentation(SCHEMA_DOCUMENTATION)
                builder.AddSimpleField(FIELD, clr.GetClrType(str))
                schema = builder.Finish()
            storage = _find_storage(doc, schema)
            if storage is None:
                storage = DataStorage.Create(doc)
            entity = Entity(schema)
            entity.Set[str](FIELD, text)
            storage.SetEntity(entity)
            transaction.Commit()
        except Exception:
            transaction.RollBack()
            raise
        return True, "stored in the model"
    except Exception as e:
        return False, "could not store it in the model: {}".format(e)


def server_of_active_document():
    """The server address kept in the model that is open now, or None. Registered with
    `server_config.set_project_reader` so an address is looked for at the moment a request is made."""
    try:
        from pyrevit import revit

        return server_of(read_text(revit.doc))
    except Exception:
        return None


def write_server(doc, url):
    """Keep `url` (None removes it) in `doc`, leaving every other setting as it was. `(ok, message)`."""
    return write_text(doc, with_server(read_text(doc), url))


def get_section(text, key):
    """The value kept under `key` in `text` (a settings object for one feature), or None."""
    value = parse(text).get(key)
    return value if isinstance(value, dict) else None


def with_section(text, key, value):
    """`text` with `key` set to `value` (a dict; None removes it); every other key is kept."""
    data = parse(text)
    data["version"] = max(VERSION, data.get("version") if isinstance(data.get("version"), int) else 0)
    if value is None:
        data.pop(key, None)
    else:
        data[key] = value
    return json.dumps(data, sort_keys=True)


def write_section(doc, key, value):
    """Keep `value` under `key` in `doc`'s settings, leaving every other setting as it was. `(ok, message)`."""
    return write_text(doc, with_section(read_text(doc), key, value))
