Every handler checks its scope: reading needs `read`, creating and updating `write`, deleting `admin`. A handler without a check is a vulnerability.
