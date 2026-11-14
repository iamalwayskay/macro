"""Copy a local stack's LocalStack objects to a directory and back.

LocalStack's community image keeps S3 objects and DynamoDB items in memory, so
a restart of its container loses uploaded files and static-file metadata. This
script runs in a one-off container of the LocalStack image, where boto3 is
installed, on the stack's network with `<dir>` mounted (see `engine`), and
copies the buckets and tables that hold user data to `<dir>` or from it.

`backup` copies new and changed objects and never deletes from `<dir>`. It
refuses to run until a `restore` has marked the running LocalStack, so a
freshly restarted, empty LocalStack never overwrites a backup.

`restore` uploads with each bucket's event notifications switched off, so
restored files do not re-enter the upload pipeline, then puts them back.
"""

import json
import os
import sys

import boto3
from botocore.exceptions import ClientError

ENDPOINT = os.environ.get("AWS_ENDPOINT_URL", "http://localstack:4566")
BUCKETS = [
    "doc-storage",
    "static-file-storage",
    "macro-email-attachments",
    "docx-upload",
    "macro-call-recording-local",
    "agent-session-changes",
]
TABLES = ["static-file-metadata"]
# A bucket that exists only once this LocalStack has been restored.
MARKER_BUCKET = "self-host-restored"


def client(service):
    return boto3.client(
        service,
        endpoint_url=ENDPOINT,
        region_name="us-east-1",
        aws_access_key_id="test",
        aws_secret_access_key="test",
    )


def is_marked(s3):
    try:
        s3.head_bucket(Bucket=MARKER_BUCKET)
        return True
    except ClientError:
        return False


def backup(root):
    s3 = client("s3")
    if not is_marked(s3):
        sys.exit("LocalStack has not been restored since it started; run restore first")

    copied = 0
    for bucket in BUCKETS:
        paginator = s3.get_paginator("list_objects_v2")
        for page in paginator.paginate(Bucket=bucket):
            for item in page.get("Contents", []):
                path = os.path.join(root, "s3", bucket, item["Key"])
                meta_path = path + ".meta.json"
                etag = item["ETag"]

                # Skip objects already copied with the same content.
                if os.path.exists(meta_path):
                    with open(meta_path) as meta_file:
                        if json.load(meta_file).get("ETag") == etag:
                            continue

                response = s3.get_object(Bucket=bucket, Key=item["Key"])
                os.makedirs(os.path.dirname(path), exist_ok=True)
                with open(path, "wb") as body_file:
                    body_file.write(response["Body"].read())
                meta = {
                    "ETag": etag,
                    "ContentType": response.get("ContentType"),
                    "Metadata": response.get("Metadata", {}),
                }
                with open(meta_path, "w") as meta_file:
                    json.dump(meta, meta_file)
                copied += 1

    dynamodb = client("dynamodb")
    for table in TABLES:
        items = []
        for page in dynamodb.get_paginator("scan").paginate(TableName=table):
            items.extend(page["Items"])
        os.makedirs(os.path.join(root, "dynamodb"), exist_ok=True)
        with open(os.path.join(root, "dynamodb", table + ".json"), "w") as table_file:
            json.dump(items, table_file)

    print(f"backup: {copied} new or changed objects")


def restore(root):
    s3 = client("s3")
    if is_marked(s3):
        print("restore: this LocalStack already holds the saved objects")
        return

    restored = 0
    for bucket in BUCKETS:
        bucket_dir = os.path.join(root, "s3", bucket)
        if not os.path.isdir(bucket_dir):
            continue

        notifications = s3.get_bucket_notification_configuration(Bucket=bucket)
        notifications.pop("ResponseMetadata", None)
        s3.put_bucket_notification_configuration(
            Bucket=bucket, NotificationConfiguration={}
        )

        for directory, _, files in os.walk(bucket_dir):
            for name in files:
                if name.endswith(".meta.json"):
                    continue
                path = os.path.join(directory, name)
                key = os.path.relpath(path, bucket_dir)
                with open(path + ".meta.json") as meta_file:
                    meta = json.load(meta_file)
                extra = {"Metadata": meta.get("Metadata") or {}}
                if meta.get("ContentType"):
                    extra["ContentType"] = meta["ContentType"]
                with open(path, "rb") as body_file:
                    s3.put_object(Bucket=bucket, Key=key, Body=body_file, **extra)
                restored += 1

        s3.put_bucket_notification_configuration(
            Bucket=bucket, NotificationConfiguration=notifications
        )

    dynamodb = client("dynamodb")
    for table in TABLES:
        path = os.path.join(root, "dynamodb", table + ".json")
        if not os.path.exists(path):
            continue
        with open(path) as table_file:
            for item in json.load(table_file):
                dynamodb.put_item(TableName=table, Item=item)

    s3.create_bucket(Bucket=MARKER_BUCKET)
    print(f"restore: {restored} objects")


if __name__ == "__main__":
    action, directory = sys.argv[1], sys.argv[2]
    {"backup": backup, "restore": restore}[action](directory)
