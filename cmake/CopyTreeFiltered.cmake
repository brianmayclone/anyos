if(NOT DEFINED SRC OR NOT DEFINED DST)
  message(FATAL_ERROR "CopyTreeFiltered.cmake requires SRC and DST")
endif()

if(NOT EXISTS "${SRC}")
  message(FATAL_ERROR "CopyTreeFiltered.cmake source does not exist: ${SRC}")
endif()

get_filename_component(_dst_parent "${DST}" DIRECTORY)

file(MAKE_DIRECTORY "${_dst_parent}")
file(REMOVE_RECURSE "${DST}")
file(COPY "${SRC}" DESTINATION "${_dst_parent}"
  PATTERN "target" EXCLUDE
  PATTERN ".fingerprint" EXCLUDE
)
