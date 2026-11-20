#!/usr/bin/env bash
# Create the off-site backup bucket and the write-only identity the backup jobs use.
#
#   AWS_PROFILE=backups-account BUCKET=acme-vectorscrub-backups REGION=us-west-2 \
#   infra/backup/aws-offsite-setup.sh
#
# Run it with credentials for a SEPARATE AWS account (or at least not the one that runs the
# database). The point of an off-site copy is that whoever takes over production cannot also
# delete the backups. What it sets up:
#   * a private bucket: public access blocked, server-side encryption on, versioning on;
#   * Object Lock in compliance mode for RETAIN_DAYS (default 35): for that long nobody, not even
#     the account's root user, can delete or overwrite a backup. That is what stops ransomware
#     or a stolen credential from erasing the history;
#   * a lifecycle rule that deletes backups after EXPIRE_DAYS (default 400) and old versions
#     soon after, so retention is bounded and the privacy policy can state it;
#   * an IAM user that can only PUT objects under two prefixes (db/ and keystore/). It cannot
#     read, list or delete. Its access key goes into /etc/vectorscrub/*.env on the backup host.
#     Restores use a different, read-only credential that you keep offline.
# It prints what it did and the access key once. It refuses to run against an existing bucket
# (Object Lock can only be turned on at creation), so it is safe to run twice.
set -euo pipefail

die() { echo "aws-offsite-setup: $*" >&2; exit 1; }
command -v aws >/dev/null || die "the aws CLI is required"
: "${BUCKET:?set BUCKET (globally unique bucket name)}"
: "${REGION:?set REGION (e.g. us-west-2)}"
RETAIN_DAYS="${RETAIN_DAYS:-35}"
EXPIRE_DAYS="${EXPIRE_DAYS:-400}"
USER_NAME="${USER_NAME:-vectorscrub-backup-writer}"
[[ "$RETAIN_DAYS" =~ ^[0-9]+$ && "$EXPIRE_DAYS" =~ ^[0-9]+$ ]] || die "RETAIN_DAYS and EXPIRE_DAYS must be whole numbers"
(( EXPIRE_DAYS > RETAIN_DAYS )) || die "EXPIRE_DAYS must be longer than RETAIN_DAYS"

account="$(aws sts get-caller-identity --query Account --output text)"
echo "aws-offsite-setup: account $account, region $REGION, bucket $BUCKET"
if aws s3api head-bucket --bucket "$BUCKET" 2>/dev/null; then
  die "bucket $BUCKET already exists; nothing changed (Object Lock can only be enabled at creation)"
fi

if [[ "$REGION" == "us-east-1" ]]; then
  aws s3api create-bucket --bucket "$BUCKET" --object-lock-enabled-for-bucket >/dev/null
else
  aws s3api create-bucket --bucket "$BUCKET" --region "$REGION" \
    --create-bucket-configuration "LocationConstraint=$REGION" --object-lock-enabled-for-bucket >/dev/null
fi
aws s3api put-public-access-block --bucket "$BUCKET" --public-access-block-configuration \
  BlockPublicAcls=true,IgnorePublicAcls=true,BlockPublicPolicy=true,RestrictPublicBuckets=true
aws s3api put-bucket-encryption --bucket "$BUCKET" --server-side-encryption-configuration \
  '{"Rules":[{"ApplyServerSideEncryptionByDefault":{"SSEAlgorithm":"AES256"},"BucketKeyEnabled":true}]}'
aws s3api put-bucket-versioning --bucket "$BUCKET" --versioning-configuration Status=Enabled
aws s3api put-object-lock-configuration --bucket "$BUCKET" --object-lock-configuration \
  "{\"ObjectLockEnabled\":\"Enabled\",\"Rule\":{\"DefaultRetention\":{\"Mode\":\"COMPLIANCE\",\"Days\":$RETAIN_DAYS}}}"
aws s3api put-bucket-lifecycle-configuration --bucket "$BUCKET" --lifecycle-configuration \
  "{\"Rules\":[{\"ID\":\"expire-backups\",\"Status\":\"Enabled\",\"Filter\":{},\"Expiration\":{\"Days\":$EXPIRE_DAYS},\"NoncurrentVersionExpiration\":{\"NoncurrentDays\":7},\"AbortIncompleteMultipartUpload\":{\"DaysAfterInitiation\":2}}]}"
# Only TLS, and nothing unencrypted.
aws s3api put-bucket-policy --bucket "$BUCKET" --policy "{
  \"Version\": \"2012-10-17\",
  \"Statement\": [
    {\"Sid\": \"TLSOnly\", \"Effect\": \"Deny\", \"Principal\": \"*\", \"Action\": \"s3:*\",
     \"Resource\": [\"arn:aws:s3:::$BUCKET\", \"arn:aws:s3:::$BUCKET/*\"],
     \"Condition\": {\"Bool\": {\"aws:SecureTransport\": \"false\"}}}
  ]
}"

aws iam create-user --user-name "$USER_NAME" >/dev/null
aws iam put-user-policy --user-name "$USER_NAME" --policy-name write-only-backups --policy-document "{
  \"Version\": \"2012-10-17\",
  \"Statement\": [
    {\"Sid\": \"PutBackupsOnly\", \"Effect\": \"Allow\", \"Action\": [\"s3:PutObject\"],
     \"Resource\": [\"arn:aws:s3:::$BUCKET/db/*\", \"arn:aws:s3:::$BUCKET/keystore/*\"]}
  ]
}"
echo "aws-offsite-setup: bucket ready. Access key for the backup host (shown once, store it in /etc/vectorscrub/*.env, mode 600):"
aws iam create-access-key --user-name "$USER_NAME" --query 'AccessKey.[AccessKeyId,SecretAccessKey]' --output text
echo "aws-offsite-setup: set BACKUP_S3_URI=s3://$BUCKET/db in backup.env and s3://$BUCKET/keystore in keystore-backup.env"
