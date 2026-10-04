#!/usr/bin/env python3
r"""Writes libesedb's reading of an ESE database as the oracle tests compare
with: every table, its columns, and every value of every record, in record
order. Run by gen.sh, which says how.

    gen.py DATABASE EXPORT_DIRECTORY > ORACLE

EXPORT_DIRECTORY is esedbexport's export of DATABASE (`-m tables`). Two
kinds of values are taken from it, the Python binding having no accessor
for them: multi-values, and compressed binary values (left compressed).

One line per table, its columns and records, tab-separated:

    table   NAME    COLUMN_COUNT    RECORD_COUNT
    columns NAME... (one per column, in order)
    types   TYPE... (JET_COLTYP numbers)
    row     VALUE...  (one per column)

A value is "-" (NULL), or KIND:DATA:

    b:HEX       a Bit: its byte (libesedb reads any but 00 as true)
    i:N         an integer (Currency included)
    f:HEX       a float or date, as the 16 hex digits of the IEEE double's
                big-endian bits (a single is widened exactly)
    x:HEX       bytes
    w:HEX       bytes of a compressed binary value, as esedbexport prints
                them: 7-bit compressed data decoded to one byte per unit
    t:TEXT      text, with backslash, tab, LF and CR escaped (\\ \t \n \r)
    j:TEXT      a multi-value as esedbexport prints it: its values as text
                joined by "; ", escaped as t:
"""

import struct
import sys

import pyesedb

BIT = 1
INTEGERS = {2, 3, 4, 5, 14, 15, 17}
FLOATS = {6, 7, 8}
TEXT = {10, 12}
COMPRESSED = 0x02


def escape(text):
    for plain, escaped in (("\\", "\\\\"), ("\t", "\\t"), ("\n", "\\n"), ("\r", "\\r")):
        text = text.replace(plain, escaped)
    return text


def float_bits(value):
    return struct.pack(">d", value).hex()


def text_or_null(text):
    return "-" if text is None else "t:" + escape(text)


def bytes_or_null(data):
    return "-" if data is None else "x:" + data.hex()


def unescape_export(text):
    """An esedbexport cell's text: it doubles backslashes."""
    return text.replace("\\\\", "\\")


def long_value(record, index, column_type):
    value = record.get_value_data_as_long_value(index)
    if column_type in TEXT:
        return text_or_null(value.get_data_as_string())
    return bytes_or_null(value.get_data())


def cell(record, index, column_type, exported):
    if record.is_long_value(index):
        return long_value(record, index, column_type)
    if record.is_multi_value(index):
        return "j:" + escape(unescape_export(exported()))
    data = record.get_value_data(index)
    if data is None:
        return "-"
    if column_type == BIT:
        return "b:%02x" % data[0]
    if column_type in INTEGERS:
        return "i:%d" % record.get_value_data_as_integer(index)
    if column_type == 8:
        # libesedb has no float accessor for dates: their 8 bytes.
        return "f:" + float_bits(struct.unpack("<d", data)[0])
    if column_type in FLOATS:
        return "f:" + float_bits(record.get_value_data_as_floating_point(index))
    if column_type in TEXT:
        return text_or_null(record.get_value_data_as_string(index))
    if record.get_value_data_flags(index) & COMPRESSED:
        return "w:" + exported()
    return bytes_or_null(data)


def exported_cells(directory, table_name, table_index):
    """esedbexport's lines of a table, split into cells, header dropped."""
    path = "%s/%s.%d" % (directory, table_name, table_index)
    with open(path, "rb") as export:
        lines = export.read().decode("utf-8").split("\n")[1:]
    return [line.split("\t") for line in lines]


def main():
    database, export_directory = sys.argv[1], sys.argv[2]
    db = pyesedb.file()
    db.open(database)
    out = sys.stdout
    for table_index, table in enumerate(db.tables):
        columns = list(table.columns)
        out.write("table\t%s\t%d\t%d\n" % (table.name, len(columns), table.number_of_records))
        out.write("\t".join(["columns"] + [column.name for column in columns]) + "\n")
        out.write("\t".join(["types"] + [str(column.type) for column in columns]) + "\n")
        export = None
        for record_index, record in enumerate(table.records):
            cells = []
            for index, column in enumerate(columns):
                def exported(record_index=record_index, index=index):
                    nonlocal export
                    if export is None:
                        export = exported_cells(export_directory, table.name, table_index)
                    return export[record_index][index]

                cells.append(cell(record, index, column.type, exported))
            out.write("\t".join(["row"] + cells) + "\n")


if __name__ == "__main__":
    main()
