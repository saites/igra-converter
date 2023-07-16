<script setup lang="ts">
defineProps<{
    value?: Object | String,
}>()
</script>

<template>
  <span v-if="value !== undefined && value !== null">{{value}}</span>
  <span v-else class="text-center">-</span>
</template>
